//! Revoke a real static grant between entry admission and flow commit.
use meshway_dataplane::{Packet, secure::SecureNode};
use meshway_static_authority::fixture::{Chain, tcp_ipv4};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tracing::{
    Event, Metadata, Subscriber,
    span::{Attributes, Id, Record},
};

fn grant(chain: &Chain) -> meshway_dataplane::secure::CircuitGrant {
    meshway_dataplane::secure::CircuitGrant {
        purpose: meshway_dataplane::Purpose::Business,
        target: chain.target.clone(),
    }
}

struct RevokeAtCommit {
    chain: Arc<Chain>,
    fired: Arc<AtomicBool>,
}

impl Subscriber for RevokeAtCommit {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "meshway_dataplane::flow::authority_boundary"
    }
    fn register_callsite(&self, _: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if self.enabled(event.metadata()) && !self.fired.swap(true, Ordering::SeqCst) {
            let origin = self.chain.path[0];
            // Entry, proof/flow preparation and this exact boundary are live.
            let snapshot = self.chain.authority.snapshot(origin).unwrap();
            snapshot.ensure_current().unwrap();
            self.chain.authority.revoke(origin).unwrap();
            assert!(snapshot.ensure_current().is_err());
        }
    }
}

#[test]
fn revoke_after_entry_before_commit_emits_no_frame_and_establishes_no_flow() {
    let chain = Arc::new(Chain::new(2, Duration::from_secs(60)).unwrap());
    let mut nodes: Vec<_> = chain
        .path
        .iter()
        .enumerate()
        .map(|(index, id)| {
            SecureNode::new(
                chain.authority.identity(*id).unwrap(),
                Default::default(),
                chain.carriers_for(index).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let init = nodes[0]
        .start_grant(grant(&chain), chain.path.clone(), &chain.carriers[0])
        .unwrap();
    let cid = init.circuit();
    let carrier = chain.carriers[0].clone();
    let response = init
        .release(|bytes| nodes[1].receive(&carrier, bytes))
        .unwrap()
        .unwrap();
    let meshway_dataplane::secure::Received::Established {
        response: Some(response),
        ..
    } = response
    else {
        panic!("response required")
    };
    response
        .release(|bytes| nodes[0].receive(&carrier, bytes))
        .unwrap()
        .unwrap();
    let packet = Packet::parse(
        tcp_ipv4(chain.addresses[0], chain.addresses[1], false, 100, 0, 2, 52).unwrap(),
    )
    .unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let subscriber = RevokeAtCommit {
        chain: chain.clone(),
        fired: fired.clone(),
    };
    assert_eq!(nodes[0].retained_flows(), 0);
    assert!(tracing::subscriber::with_default(subscriber, || nodes[0].send(cid, packet)).is_err());
    assert!(fired.load(Ordering::SeqCst));
    assert_eq!(nodes[0].retained_flows(), 0);
    assert_eq!(nodes[0].retained_circuits(), 1);
    assert!(nodes[0].ready_circuits().is_err());
    chain
        .authority
        .snapshot(chain.path[1])
        .unwrap()
        .ensure_current()
        .unwrap();
}
