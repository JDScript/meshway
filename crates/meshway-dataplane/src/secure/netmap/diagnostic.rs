use super::super::{CircuitGrant, Error, PendingFrame, SecureNode, View};
use crate::{Purpose, authority::netmap_addresses, netmap::Outcome};
use meshway_configuration::execution::{BusinessProtocol, LogicalPath};
use meshway_model::NodeId;
use meshway_policy::compile::netmap::{Target, observable};
use std::net::IpAddr;

/// Paths of a permitted `icmpEcho` decision toward `target` that every Node
/// on them can observe; `Policy` when denied, `Unsupported` when none is.
fn observable_paths(
    netmap: &meshway_configuration::netmap::CompiledNetmap,
    target: NodeId,
    outcome: &Outcome,
    path: Option<&[NodeId]>,
) -> Result<Vec<LogicalPath>, Error> {
    let Outcome::Permitted(decision) = outcome else {
        return Err(Error::Policy);
    };
    if decision.target != (Target::Mesh { node: target }) {
        return Err(Error::Policy);
    }
    let candidates: Vec<&LogicalPath> = decision
        .paths
        .iter()
        .filter(|candidate| path.is_none_or(|path| candidate.nodes == path))
        .collect();
    if candidates.is_empty() {
        return Err(Error::Policy);
    }
    let inventory = &netmap.netmap.projection.inventory;
    let paths: Vec<LogicalPath> = candidates
        .into_iter()
        .filter(|candidate| observable(inventory, &candidate.nodes))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err(Error::Unsupported);
    }
    Ok(paths)
}

/// The complete paths a `meshway ping` / observation of `target` may measure
/// on the desired netmap, decided locally like a first packet with the
/// `icmpEcho` service: no Server round trip. Denied is [`Error::Policy`];
/// permitted but with no path every Node can observe is [`Error::Unsupported`].
pub fn diagnostic_paths(
    desired: &meshway_engine::Desired,
    target: NodeId,
    path: Option<&[NodeId]>,
) -> Result<Vec<Vec<NodeId>>, Error> {
    let reading = View::of(desired);
    let netmap = reading.netmap();
    let address = mesh_address(&reading, target).ok_or(Error::Policy)?;
    let outcome = crate::netmap::Decisions::for_desired(desired)
        .decide(
            netmap,
            reading.node(),
            address,
            (BusinessProtocol::IcmpEcho, None),
        )
        .map_err(|_| Error::Policy)?;
    Ok(observable_paths(netmap, target, &outcome, path)?
        .into_iter()
        .map(|path| path.nodes)
        .collect())
}

fn mesh_address(view: &View<'_>, node: NodeId) -> Option<IpAddr> {
    let netmap = view.netmap();
    netmap_addresses(&netmap.netmap, node)
        .and_then(|a| a.ipv4.map(IpAddr::V4).or(a.ipv6.map(IpAddr::V6)))
}

impl SecureNode {
    /// Open a diagnostic circuit toward a Mesh Node, entirely locally on the
    /// first observable path of [`diagnostic_paths`] (optionally an exact one).
    /// Denied is [`Error::Policy`]; permitted but unobservable is
    /// [`Error::Unsupported`]. `None` means no next hop has an admitted carrier
    /// yet; interest is registered.
    pub fn start_diagnostic(
        &mut self,
        target: NodeId,
        path: Option<&[NodeId]>,
    ) -> Result<Option<PendingFrame>, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        let netmap = reading.netmap();
        let address = mesh_address(&reading, target).ok_or(Error::Policy)?;
        let outcome = self
            .engine
            .decisions
            .decide(
                netmap,
                reading.node(),
                address,
                (BusinessProtocol::IcmpEcho, None),
            )
            .map_err(|_| Error::Policy)?;
        let paths = observable_paths(netmap, target, &outcome, path)?;
        let grant = CircuitGrant {
            purpose: Purpose::Diagnostic,
            target: Target::Mesh { node: target },
        };
        self.open(&grant, &paths, &reading)
    }
}
