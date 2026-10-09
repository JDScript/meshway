use super::super::{
    exit::{
        command::{Commands, Tool},
        objects::{self, Receipt},
    },
    monitor::{Event, Expected},
    plan::{PROTOCOL, decode_route, decode_rule, encode_route, encode_rule, priority, route_table},
};
use super::{
    plan::{self, Ranges},
    storage::{Device, Gate, Ledger, Store},
};
use crate::{Error, Result};
use meshway_configuration::execution::EgressCapturePlan;
use rtnetlink::{
    Handle, RouteMessageBuilder,
    packet_route::{
        AddressFamily,
        route::{RouteScope, RouteType},
        rule::{RuleAction, RuleAttribute, RuleMessage},
    },
};
use serde_json::json;
use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
};

pub(super) struct Backend {
    pub store: Store,
    pub ledger: Ledger,
    pub commands: Commands,
    pub handle: Handle,
    pub expected: Arc<Mutex<Expected>>,
    pub limit: usize,
    pub start: u32,
    pub monitored: bool,
}
impl Backend {
    pub async fn initialize(&mut self) -> Result<()> {
        self.store.persist(&self.ledger).await?;
        if !self.present().await? {
            if self.ledger.receipt.is_some() {
                return Err(Error::Conflict);
            }
            let _result = objects::mutate(&mut self.commands, &self.ledger.program.create()).await;
        }
        let observed = self.dump().await?;
        if let Some(receipt) = &self.ledger.receipt {
            if observed.objects != receipt.objects || observed.handles != receipt.handles {
                return Err(Error::Conflict);
            }
            if !self.elements_match(&observed, &self.ledger.gate)
                && !self
                    .ledger
                    .pending_gate
                    .as_ref()
                    .is_some_and(|gate| self.elements_match(&observed, gate))
            {
                return Err(Error::Conflict);
            }
        } else {
            let intended = objects::decode(
                &serde_json::to_vec(&self.ledger.program.create()).map_err(|_| Error::Journal)?,
            )?;
            if observed.objects != intended.objects
                || observed.handles.len() != intended.objects.len()
            {
                #[cfg(test)]
                for (key, value) in &observed.objects {
                    if intended.objects.get(key) != Some(value) {
                        eprintln!(
                            "egress initial shape {key}: expected={:?} actual={value}",
                            intended.objects.get(key)
                        );
                        break;
                    }
                }
                return Err(Error::Conflict);
            }
            self.ledger.receipt = Some(observed);
            self.store.persist(&self.ledger).await?;
        }
        if self.monitored {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table = Some(self.ledger.table);
        }
        Ok(())
    }
    async fn present(&mut self) -> Result<bool> {
        let output = self
            .commands
            .run(
                Tool::Nft,
                vec!["-j".into(), "list".into(), "tables".into()],
                Vec::new(),
            )
            .await
            .map_err(|_| Error::Netlink)?;
        if !output.success {
            return Err(Error::Netlink);
        }
        let receipt = objects::decode(&output.stdout)?;
        Ok(receipt.objects.values().any(|table| {
            table.get("name").and_then(serde_json::Value::as_str)
                == Some(self.ledger.program.table().as_str())
                && table.get("family").and_then(serde_json::Value::as_str) == Some("inet")
        }))
    }
    async fn dump(&mut self) -> Result<Receipt> {
        self.commands.settle().await.map_err(|_| Error::Netlink)?;
        let output = self
            .commands
            .run(
                Tool::Nft,
                vec![
                    "-j".into(),
                    "-nnn".into(),
                    "list".into(),
                    "table".into(),
                    "inet".into(),
                    self.ledger.program.table(),
                ],
                Vec::new(),
            )
            .await
            .map_err(|_| Error::Netlink)?;
        if !output.success {
            return Err(Error::Conflict);
        }
        objects::decode(&output.stdout)
    }
    pub async fn verify(&mut self) -> Result<()> {
        self.verify_gate(self.ledger.gate.clone()).await
    }
    async fn verify_gate(&mut self, gate: Gate) -> Result<()> {
        let observed = self.dump().await?;
        let receipt = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if observed.objects != receipt.objects || observed.handles != receipt.handles {
            #[cfg(test)]
            for (key, value) in &observed.objects {
                if receipt.objects.get(key) != Some(value) {
                    eprintln!(
                        "egress shape {key} expected={:?} actual={value}",
                        receipt.objects.get(key)
                    );
                }
            }
            return Err(Error::Conflict);
        }
        for (name, wanted) in self.ledger.program.elements(
            &gate.ranges,
            &gate.mesh,
            gate.device.as_ref().map(|device| device.name.as_str()),
        ) {
            let key = format!("inet/{}/set/{name}", self.ledger.program.table());
            let actual = observed.elements.get(&key).cloned().unwrap_or_default();
            #[cfg(test)]
            if canonical(actual.clone()) != canonical(wanted.clone()) {
                eprintln!("egress set {name} expected={wanted:?} actual={actual:?}");
            }
            if canonical(actual) != canonical(wanted) {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
    fn elements_match(&self, observed: &Receipt, gate: &Gate) -> bool {
        self.ledger
            .program
            .elements(
                &gate.ranges,
                &gate.mesh,
                gate.device.as_ref().map(|device| device.name.as_str()),
            )
            .into_iter()
            .all(|(name, expected)| {
                let key = format!("inet/{}/set/{name}", self.ledger.program.table());
                canonical(observed.elements.get(&key).cloned().unwrap_or_default())
                    == canonical(expected)
            })
    }
    async fn gate(&mut self, gate: Gate) -> Result<()> {
        let observed = self.dump().await?;
        let receipt = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if observed.objects != receipt.objects || observed.handles != receipt.handles {
            return Err(Error::Conflict);
        }
        self.ledger.pending_gate = Some(gate.clone());
        self.store.persist(&self.ledger).await?;
        let batch = self.ledger.program.replace(
            &gate.ranges,
            &gate.mesh,
            gate.device.as_ref().map(|device| device.name.as_str()),
        );
        let _outcome = objects::mutate(&mut self.commands, &batch).await;
        self.verify_gate(gate.clone()).await?;
        self.ledger.gate = gate;
        self.ledger.pending_gate = None;
        self.store.persist(&self.ledger).await
    }
    /// Commit after the caller staged the negative union and resolved authority.
    pub async fn commit_staged(
        &mut self,
        plan: EgressCapturePlan,
        mesh: Ranges,
        device: Option<Device>,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let next = plan::parse(&plan)?;
        self.remove_rules().await?;
        self.install_rules(&next).await?;
        if let Some(device) = &device {
            self.route(device).await?;
        }
        check()?;
        if plan.unavailable.is_some() {
            return Err(Error::Target);
        }
        self.gate(Gate {
            ranges: next,
            mesh,
            device,
        })
        .await?;
        if let Err(error) = check() {
            self.pause().await?;
            return Err(error);
        }
        self.ledger.plan = plan;
        self.store.persist(&self.ledger).await
    }
    pub async fn stage(&mut self, plan: EgressCapturePlan, mesh: Ranges) -> Result<()> {
        let next = plan::parse(&plan)?;
        let mut union = plan::union(&self.ledger.gate.ranges, &next);
        if let Some(pending) = &self.ledger.pending_gate {
            union = plan::union(&union, &pending.ranges);
        }
        // First close old/new union atomically, including OUTPUT paths that
        // would otherwise hit the host local/broadcast table before the RPDB.
        self.gate(Gate {
            ranges: union.clone(),
            mesh,
            device: None,
        })
        .await?;
        self.remove_routes().await?;
        self.remove_rules().await?;
        self.install_rules(&union).await?;
        self.ledger.plan = plan;
        self.store.persist(&self.ledger).await
    }
    pub async fn pause(&mut self) -> Result<()> {
        let mut gate = self.ledger.gate.clone();
        if let Some(pending) = &self.ledger.pending_gate {
            gate.ranges = plan::union(&gate.ranges, &pending.ranges);
        }
        gate.device = None;
        self.gate(gate).await?;
        self.remove_routes().await
    }
    pub async fn retire(&mut self) -> Result<()> {
        self.pause().await?;
        self.remove_rules().await?;
        self.verify().await?;
        let _outcome = objects::mutate(&mut self.commands, &json!({"nftables": [{"delete": {"table": {"family": "inet", "name": self.ledger.program.table()}}}]})).await;
        if self.present().await? {
            return Err(Error::Conflict);
        }
        self.ledger.removed = true;
        self.store.persist(&self.ledger).await
    }
    async fn install_rules(&mut self, ranges: &Ranges) -> Result<()> {
        let mut slot = 0;
        if !ranges.v6.is_empty() {
            let end = self.start
                + u32::try_from((ranges.v4.len() + ranges.v6.len()) * 2)
                    .map_err(|_| Error::Capacity)?
                + 1;
            let mut rule = RuleMessage::default();
            rule.header.family = AddressFamily::Inet6;
            rule.header.action = RuleAction::Nop;
            rule.attributes = vec![
                RuleAttribute::Priority(end),
                RuleAttribute::Table(self.ledger.table),
                RuleAttribute::Protocol(PROTOCOL.into()),
            ];
            self.add_rule(rule).await?;
            let mut nd = RuleMessage::default();
            nd.header.family = AddressFamily::Inet6;
            nd.header.action = RuleAction::Goto;
            nd.attributes = vec![
                RuleAttribute::Priority(self.start),
                RuleAttribute::Table(self.ledger.table),
                RuleAttribute::Protocol(PROTOCOL.into()),
                RuleAttribute::Iifname("lo".into()),
                RuleAttribute::IpProtocol(58.into()),
                RuleAttribute::FwMask(self.ledger.program.mark),
                RuleAttribute::Goto(end),
            ];
            // NFT still sinks IPv6 business traffic. Its exact ND checks allow
            // only on-link control packets through the original host RPDB.
            self.add_rule(nd).await?;
            slot = 1;
        }
        for net in ranges.v4.iter().chain(&ranges.v6) {
            for action in [
                Some(RuleAction::Unreachable),
                net.addr().is_ipv4().then_some(RuleAction::ToTable),
            ]
            .into_iter()
            .flatten()
            {
                // Lookup is immediately ahead of its permanent unreachable
                // fallback. Both are local-origin and exclude protected marks.
                let offset = if action == RuleAction::ToTable { 0 } else { 1 };
                let mut rule = RuleMessage::default();
                rule.header.family = if net.addr().is_ipv4() {
                    AddressFamily::Inet
                } else {
                    AddressFamily::Inet6
                };
                rule.header.action = action;
                rule.header.dst_len = net.prefix_len();
                rule.attributes = vec![
                    RuleAttribute::Priority(self.start + slot + offset),
                    RuleAttribute::Table(self.ledger.table),
                    RuleAttribute::Protocol(PROTOCOL.into()),
                    RuleAttribute::Iifname("lo".into()),
                    RuleAttribute::FwMask(self.ledger.program.mark),
                ];
                if net.prefix_len() != 0 {
                    rule.attributes
                        .push(RuleAttribute::Destination(net.network()));
                }
                self.add_rule(rule).await?;
            }
            slot += 2;
            if slot >= plan::RESERVED {
                return Err(Error::Capacity);
            }
        }
        Ok(())
    }
    async fn add_rule(&mut self, rule: RuleMessage) -> Result<()> {
        let bytes = encode_rule(rule.clone());
        let actual = super::super::get_rules(&self.handle, self.limit).await?;
        if actual.iter().any(|other| {
            other.header.family == rule.header.family && priority(other) == priority(&rule)
        }) {
            return Err(Error::Conflict);
        }
        self.ledger.rules.push(bytes.clone());
        self.store.persist(&self.ledger).await?;
        if self.monitored {
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.egress_rules.push(bytes.clone());
            expected.add(Event {
                rule: true,
                deleted: false,
                bytes: bytes.clone(),
            })?;
        }
        let mut operation = self.handle.rule().add();
        *operation.message_mut() = rule;
        super::super::execute(operation.execute()).await?;
        if super::super::get_rules(&self.handle, self.limit)
            .await?
            .iter()
            .filter(|rule| encode_rule((*rule).clone()) == bytes)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    async fn route(&mut self, device: &Device) -> Result<()> {
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .output_interface(device.index)
            .pref_source(device.source)
            .table_id(self.ledger.table)
            .priority(100)
            .protocol(PROTOCOL.into())
            .scope(RouteScope::Link)
            .kind(RouteType::Unicast)
            .build();
        let bytes = encode_route(route.clone());
        if super::super::get_routes(&self.handle, self.limit)
            .await?
            .iter()
            .any(|route| route_table(route) == self.ledger.table)
        {
            return Err(Error::Conflict);
        }
        self.ledger.routes.push(bytes.clone());
        self.store.persist(&self.ledger).await?;
        if self.monitored {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .add(Event {
                    rule: false,
                    deleted: false,
                    bytes: bytes.clone(),
                })?;
        }
        super::super::execute(self.handle.route().add(route).execute()).await?;
        if super::super::get_routes(&self.handle, self.limit)
            .await?
            .iter()
            .filter(|route| encode_route((*route).clone()) == bytes)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub async fn remove_routes(&mut self) -> Result<()> {
        let actual = super::super::get_routes(&self.handle, self.limit).await?;
        if actual.iter().any(|route| {
            route_table(route) == self.ledger.table
                && !self.ledger.routes.contains(&encode_route(route.clone()))
        }) {
            return Err(Error::Conflict);
        }
        for bytes in self.ledger.routes.clone() {
            if actual
                .iter()
                .any(|route| encode_route(route.clone()) == bytes)
            {
                if self.monitored {
                    self.expected
                        .lock()
                        .map_err(|_| Error::Worker)?
                        .add(Event {
                            rule: false,
                            deleted: true,
                            bytes: bytes.clone(),
                        })?;
                }
                super::super::execute(self.handle.route().del(decode_route(&bytes)?).execute())
                    .await?;
            }
            if super::super::get_routes(&self.handle, self.limit)
                .await?
                .iter()
                .any(|route| encode_route(route.clone()) == bytes)
            {
                return Err(Error::Conflict);
            }
            self.ledger.routes.retain(|route| *route != bytes);
            self.store.persist(&self.ledger).await?;
        }
        Ok(())
    }
    pub async fn remove_rules(&mut self) -> Result<()> {
        for bytes in self.ledger.rules.clone().into_iter().rev() {
            let wanted = decode_rule(&bytes)?;
            let actual = super::super::get_rules(&self.handle, self.limit).await?;
            let matching: Vec<_> = actual
                .iter()
                .filter(|rule| {
                    rule.header.family == wanted.header.family
                        && priority(rule) == priority(&wanted)
                })
                .collect();
            if matching.len() > 1
                || matching
                    .first()
                    .is_some_and(|rule| encode_rule((*rule).clone()) != bytes)
            {
                return Err(Error::Conflict);
            }
            if !matching.is_empty() {
                if self.monitored {
                    self.expected
                        .lock()
                        .map_err(|_| Error::Worker)?
                        .add(Event {
                            rule: true,
                            deleted: true,
                            bytes: bytes.clone(),
                        })?;
                }
                super::super::execute(self.handle.rule().del(wanted.clone()).execute()).await?;
            }
            if super::super::get_rules(&self.handle, self.limit)
                .await?
                .iter()
                .any(|rule| {
                    rule.header.family == wanted.header.family
                        && priority(rule) == priority(&wanted)
                })
            {
                return Err(Error::Conflict);
            }
            self.ledger.rules.retain(|rule| *rule != bytes);
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_rules
                .retain(|rule| *rule != bytes);
            self.store.persist(&self.ledger).await?;
        }
        Ok(())
    }
}
fn canonical(values: Vec<serde_json::Value>) -> Vec<String> {
    let mut values: Vec<_> = values
        .into_iter()
        .map(|value| {
            if let Some(prefix) = value.get("prefix") {
                return format!(
                    "{}/{}",
                    prefix["addr"].as_str().unwrap_or(""),
                    prefix["len"]
                );
            }
            if let Some(text) = value.as_str()
                && let Ok(ip) = text.parse::<std::net::IpAddr>()
            {
                return format!("{ip}/{}", if ip.is_ipv4() { 32 } else { 128 });
            }
            value.to_string()
        })
        .collect();
    values.sort();
    values
}
