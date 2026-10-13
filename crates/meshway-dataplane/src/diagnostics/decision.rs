//! The decision `meshway ping` reports before probing: this Node's own
//! first-packet decision ([`SecureNode::preview`]) with the Policy rule that
//! decides the destination. Composition only: matching, routing and the exit
//! choice stay with their owners, and nothing here admits a packet.
use crate::secure::{Preview, SecureNode};
use meshway_configuration::execution::{BusinessProtocol, UnavailableReason};
use meshway_policy::compile::netmap::Target;
use meshway_protocol::NodeObservationTargetView;
use meshway_protocol::ipc::{PingAction, PingAssociation, PingDecision, PingResolution, PingRule};
use std::net::IpAddr;

/// The Mesh Node owning `address` among this netmap's visible Nodes.
pub fn mesh_node(secure: &SecureNode, address: IpAddr) -> Option<NodeObservationTargetView> {
    let netmap = secure.desired().netmap();
    let address = address.to_canonical();
    netmap
        .projection
        .configuration
        .nodes
        .iter()
        .find(|(id, node)| {
            **id != netmap.node_id
                && netmap.projection.inventory.contains_key(id)
                && (node.addresses.ipv4.map(IpAddr::V4) == Some(address)
                    || node.addresses.ipv6.map(IpAddr::V6) == Some(address))
        })
        .map(|(id, node)| NodeObservationTargetView {
            node_id: *id,
            hostname: node.hostname.clone(),
            addresses: node.addresses.clone(),
        })
}

/// The decision for an ICMP echo toward `destination`; `resolution` is how
/// a name target resolved (its association is filled in here).
pub fn decide(
    secure: &mut SecureNode,
    destination: IpAddr,
    resolution: Option<PingResolution>,
    now_ms: u64,
) -> PingDecision {
    let node = mesh_node(secure, destination);
    let (action, path) = match secure.preview(destination, (BusinessProtocol::IcmpEcho, None)) {
        Ok(Preview::Mesh { target, path, .. }) => (
            match target {
                Target::Mesh { .. } => PingAction::Mesh,
                Target::Exit { .. } => match path.last() {
                    Some(exit) => PingAction::Exit { node: *exit },
                    None => PingAction::Denied {
                        reason: UnavailableReason::NoPath,
                    },
                },
            },
            path,
        ),
        Ok(Preview::Direct) => (PingAction::Direct, Vec::new()),
        Ok(Preview::Unmanaged) => (PingAction::Unmanaged, Vec::new()),
        Ok(Preview::Denied(reason)) => (PingAction::Denied { reason }, Vec::new()),
        Err(_) => (
            PingAction::Denied {
                reason: UnavailableReason::MissingDependency,
            },
            Vec::new(),
        ),
    };
    let desired = secure.desired();
    let local = desired.netmap().node_id;
    let association = node
        .is_none()
        .then(|| association(desired, destination, now_ms))
        .flatten();
    let resolution = resolution.map(|resolution| PingResolution {
        association: association.clone(),
        ..resolution
    });
    let (profile, rule) = if node.is_some() {
        (None, None)
    } else {
        let compiler = desired.compiler();
        // An associated address is decided by its names, any other by its
        // address: the same split as the decision itself.
        let hit = match association.as_ref().and_then(|a| a.names.first()) {
            Some(name) => compiler.egress_rule_for_name(local, name),
            None => compiler.egress_rule_for_ip(local, destination),
        };
        let rule = hit.ok().flatten().map(|hit| PingRule {
            remark: remark(desired.netmap(), &hit.rule),
            pointer: hit.rule,
            rule_set: hit.rule_set,
        });
        let profile = meshway_policy::compile::netmap::routing_selection(desired.netmap())
            .ok()
            .map(|selection| selection.applied_profile);
        (profile, rule)
    };
    PingDecision {
        destination: Some(destination.to_string()),
        node,
        resolution,
        profile,
        rule,
        action: Some(action),
        path,
    }
}

/// A name that did not resolve: no destination, so no decision.
pub fn unresolved(resolution: PingResolution) -> PingDecision {
    PingDecision {
        destination: None,
        node: None,
        resolution: Some(resolution),
        profile: None,
        rule: None,
        action: None,
        path: Vec::new(),
    }
}

/// This Node's live associations naming `destination` under its current
/// egress selection, and the time left of the earliest one.
fn association(
    desired: &meshway_engine::Desired,
    destination: IpAddr,
    now_ms: u64,
) -> Option<PingAssociation> {
    let generation = desired.egress()?.generation;
    let local = desired.netmap().node_id;
    let destination = destination.to_canonical();
    let entries: Vec<_> = desired
        .associations()
        .entries()
        .iter()
        .filter(|entry| {
            entry.source == local
                && entry.generation == generation
                && entry.expires_at_ms > now_ms
                && entry
                    .addresses
                    .iter()
                    .any(|address| address.0.to_canonical() == destination)
        })
        .collect();
    let expires = entries.iter().map(|entry| entry.expires_at_ms).min()?;
    let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    names.sort();
    names.dedup();
    Some(PingAssociation {
        names,
        remaining_seconds: u32::try_from((expires - now_ms) / 1000).unwrap_or(u32::MAX),
    })
}

/// The remark of the egress rule at `pointer` in the netmap's configuration.
fn remark(netmap: &meshway_configuration::netmap::NodeNetmap, pointer: &str) -> String {
    let egress = &netmap.projection.configuration.routing.egress;
    let rule = |rules: &[meshway_configuration::EgressRule], index: &str| {
        index
            .parse::<usize>()
            .ok()
            .and_then(|index| rules.get(index))
            .map(|rule| rule.remark.clone())
    };
    let found = if let Some(index) = pointer.strip_prefix("/routing/egress/rules/") {
        rule(&egress.rules, index)
    } else if let Some(rest) = pointer.strip_prefix("/routing/egress/profiles/") {
        rest.rsplit_once("/rules/").and_then(|(name, index)| {
            let name = name.replace("~1", "/").replace("~0", "~");
            rule(&egress.profiles.get(&name)?.rules, index)
        })
    } else {
        None
    };
    found.unwrap_or_default()
}
