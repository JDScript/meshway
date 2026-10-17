//! Exact owned route/rule mutation shared by the capture block, the exit role
//! and source capture. Each step is: register the expected monitor event (when
//! the caller's objects are monitored), mutate, and read back the exact result.
//! Callers own their ledgers and persist intent before and outcome after a
//! step; what counts as an owned object stays the caller's predicate.
use super::{
    collect, execute, get_routes, get_rules,
    monitor::{Event, Expected},
    plan::{decode_route, decode_rule, encode_route, encode_rule, priority},
};
use crate::{Error, Result};
use rtnetlink::{
    Handle,
    packet_route::{
        link::LinkAttribute,
        route::RouteMessage,
        rule::{RuleAttribute, RuleFlags, RuleMessage},
    },
};
use std::sync::Mutex;

pub(super) struct Objects<'a> {
    pub handle: &'a Handle,
    pub expected: &'a Mutex<Expected>,
    pub limit: usize,
    /// Whether the running kernel monitor must be told about each mutation.
    /// Recovery before the monitor starts mutates unmonitored.
    pub monitored: bool,
}

fn same_slot(rule: &RuleMessage, wanted: &RuleMessage) -> bool {
    rule.header.family == wanted.header.family && priority(rule) == priority(wanted)
}

impl Objects<'_> {
    fn expect(&self, rule: bool, deleted: bool, bytes: Vec<u8>) -> Result<()> {
        if !self.monitored {
            return Ok(());
        }
        self.expected.lock().map_err(|_| Error::Worker)?.add(Event {
            rule,
            deleted,
            bytes,
        })
    }

    /// Add one rule; exactly one rule of its family and priority must then be
    /// the exact requested rule.
    pub async fn add_rule(&self, message: RuleMessage) -> Result<()> {
        let bytes = encode_rule(message.clone());
        self.expect(true, false, bytes.clone())?;
        let wanted = message.clone();
        let mut request = self.handle.rule().add();
        *request.message_mut() = message;
        execute(request.execute()).await?;
        let rules = get_rules(self.handle, self.limit).await?;
        let matching: Vec<_> = rules
            .iter()
            .filter(|rule| same_slot(rule, &wanted))
            .collect();
        if matching.len() != 1 || encode_rule(matching[0].clone()) != bytes {
            #[cfg(test)]
            eprintln!("rule readback differs: wanted={wanted:?} matching={matching:?}");
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Add one route; exactly one exact copy must then exist.
    pub async fn add_route(&self, message: RouteMessage) -> Result<()> {
        let bytes = encode_route(message.clone());
        self.expect(false, false, bytes.clone())?;
        execute(self.handle.route().add(message).execute()).await?;
        if get_routes(self.handle, self.limit)
            .await?
            .into_iter()
            .filter(|route| encode_route(route.clone()) == bytes)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Delete the exact owned route if `actual` still holds it, then prove its
    /// absence. A missing route is already deleted (a lost ACK on restart).
    pub async fn remove_route(&self, bytes: &[u8], actual: &[RouteMessage]) -> Result<()> {
        if actual
            .iter()
            .any(|route| encode_route(route.clone()) == bytes)
        {
            self.expect(false, true, bytes.to_vec())?;
            execute(self.handle.route().del(decode_route(bytes)?).execute()).await?;
        }
        if get_routes(self.handle, self.limit)
            .await?
            .iter()
            .any(|route| encode_route(route.clone()) == bytes)
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// The live rule occupying an owned rule's family and priority, if any.
    /// Anything else there, or more than one rule, is a foreign conflict. With
    /// `detached`, an observed rule on that input interface whose device is
    /// gone may carry the kernel's IIF_DETACHED state flag, which the owned
    /// record cannot.
    pub async fn owned_rule(
        &self,
        bytes: &[u8],
        detached: Option<&str>,
    ) -> Result<Option<RuleMessage>> {
        let wanted = decode_rule(bytes)?;
        let rules = get_rules(self.handle, self.limit).await?;
        let matching: Vec<_> = rules
            .iter()
            .filter(|rule| same_slot(rule, &wanted))
            .collect();
        let Some(observed) = matching.first() else {
            return Ok(None);
        };
        let mut comparable = (*observed).clone();
        if let Some(name) = detached
            && comparable
                .attributes
                .iter()
                .any(|attribute| matches!(attribute, RuleAttribute::Iifname(iif) if iif == name))
        {
            let links = collect(self.handle.link().get().execute(), self.limit).await?;
            if !links.iter().any(|link| {
                link.attributes
                    .iter()
                    .any(|attribute| matches!(attribute, LinkAttribute::IfName(n) if n == name))
            }) {
                comparable.header.flags.remove(RuleFlags::IifDetached);
            }
        }
        if encode_rule(comparable) != bytes || matching.len() > 1 {
            return Err(Error::Conflict);
        }
        Ok(Some((*observed).clone()))
    }

    /// Delete an owned rule found by [`Self::owned_rule`], using its original
    /// specification, then prove its family/priority slot is empty.
    pub async fn remove_rule(&self, bytes: &[u8], observed: Option<RuleMessage>) -> Result<()> {
        let wanted = decode_rule(bytes)?;
        if let Some(observed) = observed {
            self.expect(true, true, encode_rule(observed))?;
            // IIF_DETACHED is kernel observation state, not a userspace
            // request flag. Delete using the original owned specification.
            execute(self.handle.rule().del(wanted.clone()).execute()).await?;
        }
        if get_rules(self.handle, self.limit)
            .await?
            .iter()
            .any(|rule| same_slot(rule, &wanted))
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}
