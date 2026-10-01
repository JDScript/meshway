//! Owned Linux routing / nftables resources. Never flush ambient tables or flows.
use super::ExitPlan;
use crate::AgentError;
use std::{net::Ipv4Addr, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const TABLE: &str = "12570";
const STAGING: &str = "12572";
const BYPASS: &str = "12571";
const PRIORITY: &str = "12570";
const BYPASS_PRIORITY: &str = "12560";
const NFT: &str = "meshway_exit_p1c";
const MARK: &str = "0x4d570001";

async fn command(
    binary: &str,
    args: &[&str],
    input: Option<&str>,
    empty_ok: bool,
) -> Result<Vec<u8>, AgentError> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut child = Command::new(binary)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("piped stdin");
            stdin.write_all(input.as_bytes()).await?;
            drop(stdin);
        }
        async fn read(stream: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            stream.take(65_537).read_to_end(&mut bytes).await?;
            Ok(bytes)
        }
        let (bytes, stderr) = tokio::try_join!(
            read(child.stdout.take().expect("piped stdout")),
            read(child.stderr.take().expect("piped stderr"))
        )?;
        if bytes.len() > 65_536 || stderr.len() > 65_536 {
            return Err(AgentError::Invalid("exit command output exceeds bounds"));
        }
        let status = child.wait().await?;
        let no_entries = empty_ok
            && status.code() == Some(1)
            && String::from_utf8_lossy(&stderr).contains("0 flow entries have been deleted");
        if !status.success() && !no_entries {
            let reason = match binary {
                "nft" => {
                    "exit nftables operation failed (permission, conflict or unsupported kernel)"
                }
                "conntrack" => "exit conntrack cleanup failed (permission or unsupported kernel)",
                _ => {
                    "exit routing operation failed (permission, conflict or invalid physical route)"
                }
            };
            return Err(std::io::Error::other(format!(
                "{reason}: {}",
                String::from_utf8_lossy(&stderr)
                    .chars()
                    .take(1024)
                    .collect::<String>()
            ))
            .into());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| AgentError::Invalid("exit kernel operation timed out"))?
}

async fn ip(args: &[&str]) -> Result<Vec<u8>, AgentError> {
    command("ip", args, None, false).await
}

async fn capture(family: &str, priority: &str, table: &str) -> Result<(), AgentError> {
    // iif lo denotes locally generated traffic for policy routing. An exit
    // may also select a profile; forwarded plaintext must egress physically.
    ip(&[
        family, "rule", "add", "priority", priority, "iif", "lo", "lookup", table,
    ])
    .await?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct PhysicalRoute {
    dev: Option<String>,
    gateway: Option<Ipv4Addr>,
    prefsrc: Option<Ipv4Addr>,
}
#[derive(serde::Deserialize)]
struct Rule {
    priority: Option<u32>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TableName {
    Number(u32),
    Name(String),
}
#[derive(serde::Deserialize)]
struct ExistingRoute {
    table: Option<TableName>,
}

fn interface(name: &str) -> Result<(), AgentError> {
    if name.is_empty()
        || name.len() > 15
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(AgentError::Invalid("invalid exit interface"));
    }
    Ok(())
}

pub(crate) struct ClientRoutes {
    has_default: bool,
    bypass: bool,
    table: &'static str,
}

impl ClientRoutes {
    pub async fn install(
        plan: &ExitPlan,
        tun: &str,
        underlay: &[Ipv4Addr],
    ) -> Result<Self, AgentError> {
        interface(tun)?;
        if underlay.is_empty() || underlay.len() > 64 {
            return Err(AgentError::Invalid(
                "exit client requires bounded underlay pins",
            ));
        }
        for family in ["-4", "-6"] {
            let routes: Vec<ExistingRoute> = serde_json::from_slice(
                &ip(&["-N", "-j", family, "route", "show", "table", "all"]).await?,
            )
            .map_err(|_| AgentError::Invalid("cannot inspect exit routing tables"))?;
            if routes.iter().any(|r| match &r.table {
                Some(TableName::Number(12570..=12572)) => true,
                Some(TableName::Name(n)) => n == TABLE || n == BYPASS || n == STAGING,
                _ => false,
            }) {
                return Err(AgentError::Invalid("exit routing table already occupied"));
            }
            let rules: Vec<Rule> =
                serde_json::from_slice(&ip(&["-j", family, "rule", "show"]).await?)
                    .map_err(|_| AgentError::Invalid("cannot inspect exit rule ownership"))?;
            if rules
                .iter()
                .any(|r| matches!(r.priority, Some(12560 | 12568 | 12570)))
            {
                return Err(AgentError::Invalid("exit rule priority already occupied"));
            }
        }
        // Capture all physical lookups before installing any capture rule.
        let mut pins = Vec::new();
        for address in underlay {
            if plan.mesh.contains(address) || address.is_unspecified() || address.is_multicast() {
                return Err(AgentError::Invalid("unsafe underlay pin"));
            }
            let routes: Vec<PhysicalRoute> = serde_json::from_slice(
                &ip(&["-j", "-4", "route", "get", &address.to_string()]).await?,
            )
            .map_err(|_| AgentError::Invalid("cannot inspect physical exit route"))?;
            if routes.len() != 1 || routes[0].dev.as_deref() == Some(tun) || routes[0].dev.is_none()
            {
                return Err(AgentError::Invalid("underlay is not physically reachable"));
            }
            let route = routes.into_iter().next().expect("one route");
            interface(route.dev.as_deref().expect("validated device"))?;
            pins.push((*address, route));
        }
        for (address, route) in pins {
            let destination = format!("{address}/32");
            let mut args = vec![
                "-4".to_owned(),
                "route".into(),
                "add".into(),
                destination,
                "table".into(),
                BYPASS.into(),
            ];
            if let Some(gateway) = route.gateway {
                args.extend(["via".into(), gateway.to_string(), "onlink".into()]);
            }
            args.extend([
                "dev".into(),
                route.dev.expect("validated device"),
                "proto".into(),
                "242".into(),
            ]);
            if let Some(source) = route.prefsrc {
                args.extend(["src".into(), source.to_string()]);
            }
            ip(&args.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        }
        ip(&[
            "-4",
            "rule",
            "add",
            "priority",
            BYPASS_PRIORITY,
            "lookup",
            BYPASS,
        ])
        .await?;
        // Mesh continues to use its own stricter main-table sink and admission.
        ip(&[
            "-4",
            "route",
            "add",
            "throw",
            &plan.mesh.to_string(),
            "table",
            TABLE,
            "proto",
            "242",
        ])
        .await?;
        // Blackhole guards precede activation. They survive nonpersistent TUN death.
        let mut batch = String::new();
        for route in &plan.routes {
            batch.push_str(&format!(
                "route add unreachable {} table {TABLE} metric 42760 proto 242\n",
                route.prefix
            ));
        }
        for prefix in &plan.direct_prefixes {
            batch.push_str(&format!(
                "route add throw {prefix} table {TABLE} proto 242\n"
            ));
        }
        command("ip", &["-4", "-batch", "-"], Some(&batch), false).await?;
        let has_default = plan.routes.iter().any(|r| r.prefix.prefix_len() == 0);
        if has_default {
            ip(&[
                "-6",
                "route",
                "add",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
            capture("-6", PRIORITY, TABLE).await?;
        }
        capture("-4", PRIORITY, TABLE).await?;
        let mut batch = String::new();
        for route in &plan.routes {
            batch.push_str(&format!(
                "route add {} table {TABLE} dev {tun} src {} metric 42759 proto 242\n",
                route.prefix, plan.local
            ));
        }
        command("ip", &["-4", "-batch", "-"], Some(&batch), false).await?;
        Ok(Self {
            has_default,
            bypass: true,
            table: TABLE,
        })
    }

    /// Semantically atomic against packet I/O: caller fences old pump and drains
    /// queues before successor activation. Netlink itself has no batch transaction.
    /// Stage a complete table, publish it once, then retire only the owned old table.
    /// Failure keeps capture closed and MUST NOT reactivate the old generation.
    pub async fn replace(&mut self, plan: &ExitPlan, tun: &str) -> Result<(), AgentError> {
        interface(tun)?;
        let default = plan.routes.iter().any(|r| r.prefix.prefix_len() == 0);
        let next = if self.table == TABLE { STAGING } else { TABLE };
        let mut batch = format!("route add throw {} table {next} proto 242\n", plan.mesh);
        for route in &plan.routes {
            batch.push_str(&format!("route add unreachable {} table {next} metric 42760 proto 242\nroute add {} table {next} dev {tun} src {} metric 42759 proto 242\n",route.prefix,route.prefix,plan.local));
        }
        for prefix in &plan.direct_prefixes {
            batch.push_str(&format!(
                "route add throw {prefix} table {next} proto 242\n"
            ));
        }
        command("ip", &["-4", "-batch", "-"], Some(&batch), false).await?;
        if default && !self.has_default {
            ip(&[
                "-6",
                "route",
                "add",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
            capture("-6", PRIORITY, TABLE).await?;
        }
        capture("-4", "12568", next).await?;
        ip(&[
            "-4", "rule", "del", "priority", PRIORITY, "lookup", self.table,
        ])
        .await?;
        capture("-4", PRIORITY, next).await?;
        ip(&["-4", "rule", "del", "priority", "12568", "lookup", next]).await?;
        ip(&["-4", "route", "flush", "table", self.table, "proto", "242"]).await?;
        self.table = next;
        if !default && self.has_default {
            ip(&["-6", "rule", "del", "priority", PRIORITY, "lookup", TABLE]).await?;
            ip(&[
                "-6",
                "route",
                "del",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
        }
        self.has_default = default;
        Ok(())
    }

    /// Call only after old packet I/O has stopped. Specific rules are retained.
    pub async fn reselect(&mut self, plan: &ExitPlan, tun: &str) -> Result<(), AgentError> {
        interface(tun)?;
        let selected = plan.routes.iter().any(|r| r.prefix.prefix_len() == 0);
        if selected == self.has_default {
            return Ok(());
        }
        if selected {
            ip(&[
                "-4",
                "route",
                "add",
                "unreachable",
                "0.0.0.0/0",
                "table",
                self.table,
                "metric",
                "42760",
                "proto",
                "242",
            ])
            .await?;
            ip(&[
                "-6",
                "route",
                "add",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
            capture("-6", PRIORITY, TABLE).await?;
            ip(&[
                "-4",
                "route",
                "add",
                "0.0.0.0/0",
                "table",
                self.table,
                "dev",
                tun,
                "src",
                &plan.local.to_string(),
                "metric",
                "42759",
                "proto",
                "242",
            ])
            .await?;
        } else {
            ip(&[
                "-4",
                "route",
                "del",
                "0.0.0.0/0",
                "table",
                self.table,
                "dev",
                tun,
                "metric",
                "42759",
                "proto",
                "242",
            ])
            .await?;
            ip(&[
                "-4",
                "route",
                "del",
                "unreachable",
                "0.0.0.0/0",
                "table",
                self.table,
                "metric",
                "42760",
                "proto",
                "242",
            ])
            .await?;
            ip(&["-6", "rule", "del", "priority", PRIORITY, "lookup", TABLE]).await?;
            ip(&[
                "-6",
                "route",
                "del",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
        }
        self.has_default = selected;
        Ok(())
    }

    /// Explicit trusted cancellation of capture, never use for runtime failure.
    pub async fn restore_local(self) -> Result<(), AgentError> {
        ip(&[
            "-4", "rule", "del", "priority", PRIORITY, "lookup", self.table,
        ])
        .await?;
        if self.has_default {
            ip(&["-6", "rule", "del", "priority", PRIORITY, "lookup", TABLE]).await?;
        }
        if self.bypass {
            ip(&[
                "-4",
                "rule",
                "del",
                "priority",
                BYPASS_PRIORITY,
                "lookup",
                BYPASS,
            ])
            .await?;
        }
        for table in [self.table, BYPASS] {
            ip(&["-4", "route", "flush", "table", table, "proto", "242"]).await?;
        }
        if self.has_default {
            ip(&[
                "-6",
                "route",
                "del",
                "unreachable",
                "::/0",
                "table",
                TABLE,
                "proto",
                "242",
            ])
            .await?;
        }
        Ok(())
    }
}

/// Own the independent packet lease through teardown. No HTTP task is needed to
/// stop I/O or revoke NAT. Callers must await this before activating a successor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retirement {
    Withdraw,
    ReplaceAdmission,
}

pub(crate) async fn run(
    device: std::sync::Arc<crate::managed_dataplane::tun::Device>,
    runtime: std::sync::Arc<meshway_dataplane::Runtime>,
    plan: std::sync::Arc<ExitPlan>,
    deadline: tokio::sync::watch::Receiver<Option<tokio::time::Instant>>,
    firewall: Option<&ExitFirewall>,
    retirement: tokio::sync::watch::Receiver<Retirement>,
    activated: impl FnOnce() + Send,
) -> Result<(), AgentError> {
    let result = meshway_dataplane::packet_io::run_with_activation(
        device,
        runtime.clone(),
        super::ExitAdmission(plan),
        deadline,
        activated,
    )
    .await;
    // Only a validated same-revision selection preserves carrier state.
    let preserve =
        *retirement.borrow() == Retirement::ReplaceAdmission && retirement.has_changed().is_ok();
    if result.is_err() || !preserve {
        runtime.shutdown().await;
    }
    // The pump has cancelled both directions before touching kernel authorization.
    let cleanup = match firewall {
        Some(firewall) => firewall.withdraw().await,
        None => Ok(()),
    };
    cleanup?;
    result.map_err(AgentError::from)
}

pub(crate) struct ExitFirewall {
    tun: String,
    external: String,
    mesh: ipnet::Ipv4Net,
    previous_forwarding: bool,
}

impl ExitFirewall {
    fn deny_rules(&self) -> String {
        format!(
            "table inet {NFT} {{\nchain input {{ type filter hook input priority -10; policy accept; iifname \"{}\" drop; }}\nchain forward {{ type filter hook forward priority -10; policy accept; iifname \"{}\" drop; oifname \"{}\" drop; oifname \"{}\" drop; ip daddr {} drop; }}\n}}\n",
            self.tun, self.tun, self.tun, self.external, self.mesh
        )
    }

    pub async fn install(plan: &ExitPlan, tun: &str, external: &str) -> Result<Self, AgentError> {
        interface(tun)?;
        interface(external)?;
        if tun == external || plan.grants.is_empty() {
            return Err(AgentError::Invalid(
                "exit requires distinct interfaces and explicit grants",
            ));
        }
        // Check existence via bounded JSON; a failed list is not proof of absence.
        #[derive(serde::Deserialize)]
        struct Tables {
            nftables: Vec<Entry>,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            table: Option<Table>,
        }
        #[derive(serde::Deserialize)]
        struct Table {
            name: String,
        }
        let tables: Tables =
            serde_json::from_slice(&command("nft", &["-j", "list", "tables"], None, false).await?)
                .map_err(|_| AgentError::Invalid("cannot inspect nft ownership"))?;
        if tables
            .nftables
            .iter()
            .any(|entry| entry.table.as_ref().is_some_and(|t| t.name == NFT))
        {
            return Err(AgentError::Invalid(
                "exit firewall already exists; refusing adoption",
            ));
        }
        let firewall = Self {
            tun: tun.into(),
            external: external.into(),
            mesh: plan.mesh,
            previous_forwarding: std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim()
                == "1",
        };
        // Establish rejection before enabling NAT. Failed activation stays closed.
        command("nft", &["-f", "-"], Some(&firewall.deny_rules()), false).await?;
        if !firewall.previous_forwarding {
            std::fs::write("/proc/sys/net/ipv4/ip_forward", b"1\n").map_err(|_| {
                AgentError::Invalid(
                    "exit IPv4 forwarding cannot be enabled (permission or read-only sysctl)",
                )
            })?;
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            return Err(AgentError::Invalid(
                "exit IPv4 forwarding did not become active",
            ));
        }
        // Mark is reserved only within this owned namespace/prototype.
        if !command("conntrack", &["-L", "--mark", MARK], None, false)
            .await?
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            return Err(AgentError::Invalid(
                "exit conntrack mark already occupied; refusing to delete unowned state",
            ));
        }
        firewall.activate(plan).await?;
        Ok(firewall)
    }

    async fn activate(&self, plan: &ExitPlan) -> Result<(), AgentError> {
        let tun = &self.tun;
        let external = &self.external;
        let mut forwards = String::new();
        for grant in &plan.grants {
            for prefix in &grant.prefixes {
                let common = format!(
                    "iifname \"{tun}\" oifname \"{external}\" ip saddr {} ip daddr {prefix} ct mark {MARK}",
                    grant.source
                );
                forwards.push_str(&format!("{common} ct state established accept;\n{common} ct state new ip protocol tcp tcp flags & (fin | syn | rst | ack) == syn accept;\n{common} ct state new ip protocol udp accept;\n{common} ct state new ip protocol icmp icmp type echo-request accept;\n"));
            }
        }
        let rules = format!(
            "delete table inet {NFT}\ntable inet {NFT} {{\nchain prerouting {{ type filter hook prerouting priority -150; policy accept; iifname \"{tun}\" ct mark set {MARK}; }}\nchain input {{ type filter hook input priority -10; policy accept; iifname \"{tun}\" drop; }}\nchain forward {{ type filter hook forward priority -10; policy accept; {forwards} iifname \"{external}\" oifname \"{tun}\" ct mark {MARK} ct state established accept; iifname \"{tun}\" drop; oifname \"{tun}\" drop; oifname \"{external}\" drop; ip daddr {} drop; }}\nchain nat {{ type nat hook postrouting priority 100; policy accept; oifname \"{external}\" ct mark {MARK} masquerade; }}\n}}\n",
            plan.mesh
        );
        command("nft", &["-f", "-"], Some(&rules), false).await?;
        Ok(())
    }

    /// Reuse only this live ownership token after fencing the previous pump.
    /// A crash's orphan table is never adopted just because its name matches.
    pub async fn replace(&self, plan: &ExitPlan) -> Result<(), AgentError> {
        if plan.mesh != self.mesh {
            return Err(AgentError::Invalid("exit network binding changed"));
        }
        self.withdraw().await?;
        if plan.grants.is_empty() {
            return Ok(());
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            std::fs::write("/proc/sys/net/ipv4/ip_forward", b"1\n")
                .map_err(|_| AgentError::Invalid("exit IPv4 forwarding cannot be re-enabled"))?;
        }
        if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() != "1" {
            return Err(AgentError::Invalid(
                "exit IPv4 forwarding did not become active",
            ));
        }
        self.activate(plan).await
    }

    /// Rejection is committed before deleting state. A failed deletion stays closed.
    pub async fn withdraw(&self) -> Result<(), AgentError> {
        let rules = format!("delete table inet {NFT}\n{}", self.deny_rules());
        command("nft", &["-f", "-"], Some(&rules), false).await?;
        command("conntrack", &["-D", "--mark", MARK], None, true).await?;
        if !self.previous_forwarding
            && std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")?.trim() == "1"
        {
            std::fs::write("/proc/sys/net/ipv4/ip_forward", b"0\n")
                .map_err(|_| AgentError::Invalid("exit IPv4 forwarding restore failed"))?;
        }
        Ok(())
    }
}
