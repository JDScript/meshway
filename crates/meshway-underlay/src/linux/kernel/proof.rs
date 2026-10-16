//! Read-back of the owned rules, routes, capture and TUN against the
//! journal before and after each change.
use super::*;

impl Kernel {
    pub(super) async fn ready(&mut self) -> Result<()> {
        self.store.settle().await?;
        self.shared.check()?;
        if self.journal.pending.is_some() {
            return Err(Error::Journal);
        }
        Ok(())
    }

    pub(super) async fn verify(&self) -> Result<()> {
        if let Some(anchor) = &self.journal.anchor {
            let capture = self
                .expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table;
            anchor::verify(
                &self.handle,
                anchor,
                capture,
                self.options.max_kernel_objects,
            )
            .await?;
        }
        let mut rules = get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let exit_rules = {
            let expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected
                .exit_rules
                .iter()
                .chain(&expected.egress_rules)
                .cloned()
                .collect::<Vec<_>>()
        };
        // An exit rule whose input TUN is gone carries the kernel's
        // IIF_DETACHED state flag; it is still that owner's own rule.
        rules.retain(|rule| {
            let mut attached = rule.clone();
            attached
                .header
                .flags
                .remove(rtnetlink::packet_route::rule::RuleFlags::IifDetached);
            !exit_rules.contains(&encode_rule(rule.clone()))
                && !exit_rules.contains(&encode_rule(attached))
        });
        verify_rules(&self.journal, &rules)?;
        let mut foreign: Vec<_> = rules
            .into_iter()
            .filter(|rule| {
                !self
                    .journal
                    .rules
                    .iter()
                    .any(|record| record.observed == encode_rule(rule.clone()))
            })
            .map(encode_rule)
            .collect();
        foreign.sort();
        if foreign != self.baseline {
            return Err(Error::Conflict);
        }
        let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
        // Removal and repair proceed while an external address overlaps a
        // pool; only installation refuses it.
        self.verify_capture_routes(&self.journal.pools, &routes, true)
            .await?;
        let actual: Vec<_> = routes
            .into_iter()
            .filter(|route| route_table(route) == self.journal.table)
            .map(encode_route)
            .collect();
        if actual.len() != self.journal.routes.len()
            || self.journal.routes.iter().any(|record| {
                actual
                    .iter()
                    .filter(|bytes| **bytes == record.observed)
                    .count()
                    != 1
            })
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    pub(super) async fn verify_capture(&self, pools: &[ipnet::IpNet]) -> Result<()> {
        let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
        self.verify_capture_routes(pools, &routes, false).await
    }

    async fn verify_capture_routes(
        &self,
        pools: &[ipnet::IpNet],
        routes: &[RouteMessage],
        overlap: bool,
    ) -> Result<()> {
        let addresses = get_addresses(&self.handle, self.options.max_kernel_objects).await?;
        let mut assigned = Vec::new();
        let owned = if let Some(tun) = &self.tun {
            let record = self.journal.tun.as_ref().ok_or(Error::Conflict)?;
            let index = tun.if_index()?;
            if index == 0 || index != record.ifindex || tun.name() != record.name {
                return Err(Error::Conflict);
            }
            assigned.extend(
                [
                    record.addresses.ipv4.map(IpAddr::V4),
                    record.addresses.ipv6.map(IpAddr::V6),
                ]
                .into_iter()
                .flatten(),
            );
            Some(capture::Owned {
                index,
                addresses: &assigned,
            })
        } else {
            None
        };
        let expected = self.expected.lock().map_err(|_| Error::Worker)?;
        let tolerated = |route: &RouteMessage| overlap && monitor::overlap_route(&expected, route);
        if routes
            .iter()
            .any(|route| capture::route_conflicts(pools, owned, route) && !tolerated(route))
            || addresses.iter().any(|address| {
                capture::address_conflicts(pools, owned, address)
                    && !(overlap && expected.overlapping(address))
            })
        {
            return Err(Error::Conflict);
        }
        drop(expected);
        self.shared.check()
    }

    pub(super) async fn verify_tun(
        &self,
        tun: &Tun,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<()> {
        let record = self.journal.tun.as_ref().ok_or(Error::Conflict)?;
        let index = tun.if_index()?;
        if record.ifindex != index
            || record.name != tun.name()
            || record.mtu != self.options.mtu
            || tun.mtu() != self.options.mtu
            || record.addresses != *addresses
            || !live_routes_match(&self.journal, index, pools, addresses)
        {
            return Err(Error::Conflict);
        }
        let links = collect(
            self.handle.link().get().match_index(index).execute(),
            self.options.max_kernel_objects,
        )
        .await?;
        if links.len() != 1 {
            return Err(Error::Conflict);
        }
        let link = &links[0];
        use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags};
        if link.header.index != index
            || !link.header.flags.contains(LinkFlags::Up)
            || !link
                .attributes
                .iter()
                .any(|a| matches!(a, LinkAttribute::IfName(name) if name == tun.name()))
            || !link
                .attributes
                .iter()
                .any(|a| matches!(a, LinkAttribute::Mtu(mtu) if *mtu as usize == self.options.mtu))
        {
            return Err(Error::Conflict);
        }
        let actual = get_addresses(&self.handle, self.options.max_kernel_objects).await?;
        // Verification is a single bounded dump, not a new readiness grace
        // period. The Control handoff's original deadlines remain authoritative.
        for wanted in [
            addresses.ipv4.map(IpAddr::V4),
            addresses.ipv6.map(IpAddr::V6),
        ]
        .into_iter()
        .flatten()
        {
            let matches: Vec<_> = actual.iter().filter(|address| address.header.index == index
                && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(ip) | AddressAttribute::Local(ip) if *ip == wanted))).collect();
            if matches.len() != 1 {
                return Err(Error::Conflict);
            }
            let address = matches[0];
            let flags = network::address_flags(address);
            if flags & 0xc8 != 0x80
                || address.header.prefix_len != if wanted.is_ipv4() { 32 } else { 128 }
            {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
}

pub(super) fn verify_rules(journal: &Journal, rules: &[RuleMessage]) -> Result<()> {
    for record in &journal.rules {
        let matching: Vec<_> = rules
            .iter()
            .filter(|rule| {
                rule.header.family == record.spec.message().header.family
                    && priority(rule) == record.spec.priority
            })
            .collect();
        if matching.len() != 1 || encode_rule(matching[0].clone()) != record.observed {
            return Err(Error::Conflict);
        }
    }
    for rule in rules {
        if ((journal.start..=journal.end()).contains(&priority(rule))
            || rule_table(rule) == journal.table)
            && !journal
                .rules
                .iter()
                .any(|record| record.observed == encode_rule(rule.clone()))
        {
            return Err(Error::Conflict);
        }
        if rule.attributes.iter().any(|a| matches!(a, RuleAttribute::Goto(target) if (journal.start..=journal.end()).contains(target)))
            && !journal.rules.iter().any(|record| record.observed == encode_rule(rule.clone())) { return Err(Error::Conflict); }
    }
    Ok(())
}

pub(super) fn live_routes_match(
    journal: &Journal,
    index: u32,
    pools: &[ipnet::IpNet],
    addresses: &meshway_configuration::NodeAddresses,
) -> bool {
    let expected = plan::positive_routes(pools, addresses, journal.table, index);
    journal.routes.len() == expected.len()
        && expected.iter().all(|route| {
            journal
                .routes
                .iter()
                .filter(|record| record.spec == *route)
                .count()
                == 1
        })
}
