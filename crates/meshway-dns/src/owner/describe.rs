//! The DNS path of one name, read-only: the same selection and routes
//! [`Owner::begin_published_until`] takes, without a ticket or any I/O.
use super::*;
use meshway_model::NodeId;

/// Where this Node's own query for a name goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameRoute {
    /// Answered from the netmap's MagicDNS names.
    MagicDns,
    /// The exit's resolver service over the Mesh.
    Exit(NodeId),
    /// Configured resolvers by key, first one with a local route.
    Resolvers(Vec<String>),
    /// Ordinary host resolution.
    System,
    /// No path: the query fails locally.
    Unavailable,
}

/// The route of an `A` query for `name` on `desired`.
pub fn name_route(desired: &Desired, name: &str) -> NameRoute {
    let Ok(published) = PublishedDns::from_netmap(desired.netmap()) else {
        return NameRoute::Unavailable;
    };
    let Some(query) = query(name) else {
        return NameRoute::Unavailable;
    };
    let plan = Plan::of(desired);
    let exit = if matches!(plan, Plan::NoExit | Plan::ServeExitResolver) {
        ExitSelection::NoExit
    } else {
        ExitSelection::Exit
    };
    match published.select(&query, exit) {
        Selection::Published(_) => NameRoute::MagicDns,
        Selection::ExitResolver => match &desired.netmap().dns.egress {
            meshway_configuration::netmap::DnsPlan::ViaExit { service } => {
                NameRoute::Exit(service.exit)
            }
            _ => NameRoute::Unavailable,
        },
        Selection::Resolvers(candidates) => {
            let deadline = Instant::now() + Duration::from_secs(1);
            match candidates.first() {
                Some(first)
                    if plan != Plan::Unavailable
                        && LocalResolverRoute::new(desired, first.key, deadline).is_ok() =>
                {
                    NameRoute::Resolvers(
                        candidates
                            .iter()
                            .map(|candidate| candidate.key.to_owned())
                            .collect(),
                    )
                }
                _ => NameRoute::Unavailable,
            }
        }
        Selection::Unmanaged => NameRoute::System,
        Selection::Unavailable(_) => NameRoute::Unavailable,
    }
}

fn query(name: &str) -> Option<Query> {
    use hickory_proto::op::{MessageType, OpCode, Query as Question};
    let mut name = hickory_proto::rr::Name::from_ascii(name).ok()?;
    name.set_fqdn(true);
    let mut message = Message::new(0, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message.add_query(Question::query(name, RecordType::A));
    crate::decode_query(&message.to_vec().ok()?).ok()
}
