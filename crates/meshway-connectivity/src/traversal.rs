//! Host gateway contract. Calls enter the host's existing Control queue; this
//! library never owns an HTTP client, credential, or another State store.
use crate::Error;
use futures_util::future::BoxFuture;
use meshway_authority::ExecutionSnapshot;
use meshway_control::connectivity::{
    AttemptView, Discovery, DiscoveryContext, Outcome, ValidatedOffer,
};
use meshway_model::NodeId;
use meshway_protocol::{
    StateView,
    connectivity::{CandidateReference, PrepareUdpTraversalRequest},
};

pub enum Operation {
    Query(Box<ExecutionSnapshot>),
    Peers {
        context: DiscoveryContext,
        peers: Vec<NodeId>,
    },
    Prepare {
        context: DiscoveryContext,
        request: Box<PrepareUdpTraversalRequest>,
    },
    Accept(ValidatedOffer),
    Cancel(ValidatedOffer),
}
pub enum Reply {
    Query(Outcome<Discovery>),
    Peers(StateView),
    Prepare(Outcome<ValidatedOffer>),
    Accept(Outcome<AttemptView>),
    Cancelled,
}
pub trait TraversalGateway: Send + Sync + 'static {
    fn request(&self, operation: Operation) -> BoxFuture<'_, Result<Reply, Error>>;
    /// True only for the exact observation in an acknowledged State report.
    fn published(&self, candidate: &CandidateReference) -> bool;
}
