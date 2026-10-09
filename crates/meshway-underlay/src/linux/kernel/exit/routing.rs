use super::super::{
    monitor::Expected,
    plan::{PROTOCOL, decode_route, decode_rule, encode_route, encode_rule, priority},
};
use super::storage::{Ledger, Store};
use crate::{Error, Result};
use rtnetlink::{
    Handle, RouteMessageBuilder,
    packet_route::{
        AddressFamily,
        route::{RouteScope, RouteType},
        rule::{RuleAction, RuleAttribute, RuleMessage},
    },
};
use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
};

pub(in crate::linux::kernel) const RESERVED: u32 = 512;

pub(super) struct Routing {
    pub handle: Handle,
    pub expected: Arc<Mutex<Expected>>,
    pub limit: usize,
    pub start: u32,
    pub end: u32,
}

impl Routing {
    fn objects(&self, monitored: bool) -> super::super::rpdb::Objects<'_> {
        super::super::rpdb::Objects {
            handle: &self.handle,
            expected: &self.expected,
            limit: self.limit,
            monitored,
        }
    }

    pub async fn forward(&self, ledger: &mut Ledger, store: &mut Store) -> Result<()> {
        if self.start <= RESERVED {
            return Err(Error::Conflict);
        }
        let mut rule = RuleMessage::default();
        rule.header.family = AddressFamily::Inet;
        rule.header.action = RuleAction::Goto;
        rule.attributes = vec![
            RuleAttribute::Priority(self.start - RESERVED),
            RuleAttribute::Table(ledger.route_table),
            RuleAttribute::Protocol(PROTOCOL.into()),
            RuleAttribute::FwMark(ledger.program.mark),
            RuleAttribute::FwMask(ledger.program.mark),
            RuleAttribute::Iifname(ledger.tun_name.clone()),
            RuleAttribute::Goto(self.end),
        ];
        self.add_rule(rule, ledger, store).await
    }

    pub async fn source(
        &self,
        source: Ipv4Addr,
        remote: Ipv4Addr,
        ledger: &mut Ledger,
        store: &mut Store,
    ) -> Result<()> {
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(source, 32)
            .output_interface(ledger.program.interface)
            .table_id(ledger.route_table)
            .priority(100)
            .protocol(PROTOCOL.into())
            .scope(RouteScope::Link)
            .kind(RouteType::Unicast)
            .build();
        let bytes = encode_route(route.clone());
        if !ledger.routes.contains(&bytes) {
            ledger.pending = true;
            ledger.routes.push(bytes.clone());
            store.persist(ledger).await?;
            self.objects(true).add_route(route).await?;
            ledger.pending = false;
            store.persist(ledger).await?;
        }
        let rules = super::super::get_rules(&self.handle, self.limit).await?;
        if ledger.rules.iter().any(|bytes| {
            decode_rule(bytes).is_ok_and(|rule| {
                rule.header.action == RuleAction::ToTable
                    && rule
                        .attributes
                        .contains(&RuleAttribute::Source(remote.into()))
                    && rule
                        .attributes
                        .contains(&RuleAttribute::Destination(source.into()))
            })
        }) {
            return Ok(());
        }
        let base = (0..super::program::CAPACITY)
            .map(|index| self.start - RESERVED + 1 + index as u32 * 2)
            .find(|base| {
                !rules
                    .iter()
                    .any(|rule| [*base, *base + 1].contains(&priority(rule)))
            })
            .ok_or(Error::Capacity)?;
        for (offset, action) in [(0, RuleAction::ToTable), (1, RuleAction::Unreachable)] {
            let mut rule = RuleMessage::default();
            rule.header.family = AddressFamily::Inet;
            rule.header.action = action;
            rule.header.src_len = 32;
            rule.header.dst_len = 32;
            // An ordinary local socket never has the permitted external peer's
            // source address. No mark-only /0 lookup can hijack bootstrap I/O.
            rule.attributes = vec![
                RuleAttribute::Priority(base + offset),
                RuleAttribute::Table(ledger.route_table),
                RuleAttribute::Protocol(PROTOCOL.into()),
                RuleAttribute::FwMark(ledger.program.mark),
                RuleAttribute::FwMask(ledger.program.mark),
                RuleAttribute::Source(remote.into()),
                RuleAttribute::Destination(source.into()),
            ];
            self.add_rule(rule, ledger, store).await?;
        }
        Ok(())
    }

    async fn add_rule(
        &self,
        message: RuleMessage,
        ledger: &mut Ledger,
        store: &mut Store,
    ) -> Result<()> {
        let bytes = encode_rule(message.clone());
        if ledger.rules.contains(&bytes) {
            return Ok(());
        }
        ledger.pending = true;
        ledger.rules.push(bytes.clone());
        store.persist(ledger).await?;
        self.expected
            .lock()
            .map_err(|_| Error::Worker)?
            .exit_rules
            .push(bytes.clone());
        self.objects(true).add_rule(message).await?;
        ledger.pending = false;
        store.persist(ledger).await
    }

    pub async fn remove(
        &self,
        ledger: &mut Ledger,
        store: &mut Store,
        monitored: bool,
    ) -> Result<()> {
        self.remove_selected(
            ledger,
            store,
            monitored,
            ledger.routes.clone(),
            ledger.rules.clone(),
        )
        .await
    }

    pub async fn reclaim(
        &self,
        tuple: super::tuple::OriginalTuple,
        ledger: &mut Ledger,
        store: &mut Store,
    ) -> Result<()> {
        let source_used = ledger
            .projections
            .iter()
            .any(|flow| !flow.retired && flow.tuple.source == tuple.source);
        let pair_used = ledger.projections.iter().any(|flow| {
            !flow.retired
                && flow.tuple.source == tuple.source
                && flow.tuple.destination == tuple.destination
        });
        let routes = if source_used {
            Vec::new()
        } else {
            ledger
                .routes
                .iter()
                .filter(|bytes| {
                    decode_route(bytes).is_ok_and(|route| {
                        route.attributes.contains(
                            &rtnetlink::packet_route::route::RouteAttribute::Destination(
                                tuple.source.into(),
                            ),
                        )
                    })
                })
                .cloned()
                .collect()
        };
        let rules = if pair_used {
            Vec::new()
        } else {
            ledger
                .rules
                .iter()
                .filter(|bytes| {
                    decode_rule(bytes).is_ok_and(|rule| {
                        rule.attributes
                            .contains(&RuleAttribute::Source(tuple.destination.into()))
                            && rule
                                .attributes
                                .contains(&RuleAttribute::Destination(tuple.source.into()))
                    })
                })
                .cloned()
                .collect()
        };
        self.remove_selected(ledger, store, true, routes, rules)
            .await
    }

    async fn remove_selected(
        &self,
        ledger: &mut Ledger,
        store: &mut Store,
        monitored: bool,
        routes: Vec<Vec<u8>>,
        rules: Vec<Vec<u8>>,
    ) -> Result<()> {
        let actual_routes = super::super::get_routes(&self.handle, self.limit).await?;
        if actual_routes.iter().any(|route| {
            super::super::plan::route_table(route) == ledger.route_table
                && !ledger.routes.contains(&encode_route(route.clone()))
        }) {
            return Err(Error::Conflict);
        }
        let objects = self.objects(monitored);
        for bytes in routes {
            ledger.pending = true;
            store.persist(ledger).await?;
            objects.remove_route(&bytes, &actual_routes).await?;
            ledger.routes.retain(|route| *route != bytes);
            store.persist(ledger).await?;
        }
        for bytes in rules.into_iter().rev() {
            let observed = objects.owned_rule(&bytes, Some(&ledger.tun_name)).await?;
            ledger.pending = true;
            store.persist(ledger).await?;
            objects.remove_rule(&bytes, observed).await?;
            ledger.rules.retain(|rule| *rule != bytes);
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .exit_rules
                .retain(|rule| *rule != bytes);
            ledger.pending = false;
            store.persist(ledger).await?;
        }
        ledger.pending = false;
        store.persist(ledger).await?;
        Ok(())
    }
}
