//! HTTP authorization and typed Node self-reports, not packet-flow evidence.
use super::*;
use meshway_protocol::node_observations::*;

fn demand(a: &Node, target: &str) -> CreateNodeObservation {
    CreateNodeObservation {
        instance: a.instance.clone(),
        subscription_id: id(),
        target: NodeObservationTarget::Name {
            name: target.into(),
        },
        service: ObservationServiceScope::Tcp {
            port: 443.try_into().unwrap(),
        },
        selection: ObservationPath::Auto,
        metrics: vec![MetricKind::Rtt, MetricKind::Traffic],
        interval_ms: 1000,
    }
}
fn selected(request: &CreateNodeObservation) -> String {
    format!("/agent-api/v1/observations/{}", request.subscription_id)
}
fn query(a: &Node) -> NodeObservationQuery {
    NodeObservationQuery {
        instance_id: a.instance.instance_id,
        generation: a.instance.generation,
    }
}

mod capabilities;

mod selectors;
