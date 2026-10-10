use super::*;
use std::{future::pending, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Packets,
    Adjacencies,
    Host,
    Dns,
}
#[derive(Default)]
struct Resources {
    calls: Vec<(Part, Option<NetmapRef>)>,
    fail: Option<Part>,
    block: Option<Part>,
}
impl Resources {
    async fn apply(&mut self, part: Part, desired: Option<&Desired>) -> Result<(), Error> {
        self.calls.push((part, desired.map(Desired::reference)));
        if desired.is_some() && self.block == Some(part) {
            pending::<()>().await;
        }
        if desired.is_some() && self.fail == Some(part) {
            Err(Error::Apply)
        } else {
            Ok(())
        }
    }
}
impl Components for Resources {
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Packets, desired).await
    }
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Adjacencies, desired).await
    }
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Host, desired).await
    }
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Dns, desired).await
    }
}
fn input(
    engine: &mut Engine<Resources>,
    content: Arc<CompiledNetmap>,
    milliseconds: u64,
) -> Netmap {
    let now = engine.clock.now();
    Netmap {
        lease: NetmapLease::new(content.reference(), now, now + milliseconds).unwrap(),
        content,
    }
}

#[tokio::test(start_paused = true)]
async fn renewal_changes_only_engine_timer_and_expiry_withdraws_every_component() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_netmap_fixture::single_node("a", None));
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let compiler = engine.desired().unwrap().compiler().clone();
    let before = engine.next_expiry().unwrap();
    let calls = engine.components.calls.clone();
    tokio::time::advance(Duration::from_secs(20)).await;
    let renewed = input(&mut engine, content, 60_000);
    engine.reconfig(Some(&renewed)).await.unwrap();
    assert!(engine.next_expiry().unwrap() > before);
    assert!(Arc::ptr_eq(&compiler, engine.desired().unwrap().compiler()));
    assert_eq!(engine.components.calls, calls);
    tokio::time::advance(Duration::from_secs(61)).await;
    engine.expire().await.unwrap();
    assert!(engine.desired().is_none() && engine.next_expiry().is_none());
    assert_eq!(
        &engine.components.calls[calls.len()..],
        &[
            (Part::Packets, None),
            (Part::Adjacencies, None),
            (Part::Host, None),
            (Part::Dns, None),
        ]
    );
    // A fresh signed lease re-installs the same content after actual expiry.
    let restored = input(&mut engine, renewed.content, 60_000);
    engine.reconfig(Some(&restored)).await.unwrap();
    assert!(engine.desired().is_some());
    assert_eq!(engine.components.calls.len(), calls.len() + 8);
}

#[tokio::test(start_paused = true)]
async fn failed_or_cancelled_apply_never_exposes_old_desired_as_current() {
    let content = Arc::new(meshway_netmap_fixture::single_node("a", None));
    for part in [Part::Packets, Part::Adjacencies, Part::Host, Part::Dns] {
        let mut engine = Engine::new(Resources {
            fail: Some(part),
            ..Default::default()
        });
        let first = input(&mut engine, content.clone(), 60_000);
        assert_eq!(engine.reconfig(Some(&first)).await, Err(Error::Apply));
        assert!(engine.desired().is_none());
        assert!(engine.components.calls.ends_with(&[
            (Part::Packets, None),
            (Part::Adjacencies, None),
            (Part::Host, None),
            (Part::Dns, None),
        ]));
    }
    let mut engine = Engine::new(Resources::default());
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let next = input(
        &mut engine,
        Arc::new(meshway_netmap_fixture::single_node("b", Some(&content))),
        60_000,
    );
    engine.components.block = Some(Part::Host);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), engine.reconfig(Some(&next)))
            .await
            .is_err()
    );
    assert!(engine.desired().is_none() && engine.next_expiry().is_none());
    engine.components.block = None;
    engine.reconfig(None).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn older_signed_lease_and_regressing_content_do_not_change_live_engine() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_netmap_fixture::single_node("a", None));
    let first = input(&mut engine, content, 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let calls = engine.components.calls.clone();
    let expires = engine.next_expiry();
    let mut older = Netmap {
        content: first.content.clone(),
        lease: first.lease,
    };
    older.lease.expires_at_ms -= 1;
    assert_eq!(engine.reconfig(Some(&older)).await, Err(Error::Reference));
    assert_eq!(engine.next_expiry(), expires);
    assert_eq!(engine.components.calls, calls);
    // A Server clock stepped back issues an earlier `issuedAt`; a lease that
    // does not end earlier is still a renewal.
    let mut stepped = Netmap {
        content: first.content.clone(),
        lease: first.lease,
    };
    stepped.lease.issued_at_ms -= 5_000;
    stepped.lease.expires_at_ms += 1_000;
    engine.reconfig(Some(&stepped)).await.unwrap();
    assert!(engine.next_expiry() > expires);
    assert_eq!(engine.components.calls, calls);
}

#[derive(Default)]
struct Degrading {
    calls: Vec<(Part, Option<NetmapRef>)>,
    degraded: bool,
}
impl Degrading {
    fn apply(&mut self, part: Part, desired: Option<&Desired>) -> Result<(), Error> {
        self.calls.push((part, desired.map(Desired::reference)));
        if desired.is_some() && part == Part::Adjacencies && self.degraded {
            Err(Error::Degraded)
        } else {
            Ok(())
        }
    }
}
impl Components for Degrading {
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Packets, desired)
    }
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Adjacencies, desired)
    }
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Host, desired)
    }
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Dns, desired)
    }
}

#[tokio::test(start_paused = true)]
async fn a_degraded_apply_keeps_content_and_the_same_content_retries_it() {
    let mut engine = Engine::new(Degrading {
        degraded: true,
        ..Default::default()
    });
    let content = Arc::new(meshway_netmap_fixture::single_node("a", None));
    let now = engine.clock.now();
    let first = Netmap {
        lease: NetmapLease::new(content.reference(), now, now + 60_000).unwrap(),
        content,
    };
    assert_eq!(engine.reconfig(Some(&first)).await, Err(Error::Degraded));
    let reference = Some(first.content.reference());
    assert!(engine.degraded() && engine.next_expiry().is_some());
    assert_eq!(engine.desired().map(Desired::reference), reference);
    // Every step ran on the content; nothing was withdrawn.
    assert_eq!(
        engine.components.calls,
        [Part::Host, Part::Adjacencies, Part::Packets, Part::Dns].map(|part| (part, reference))
    );
    engine.components.calls.clear();
    engine.components.degraded = false;
    engine.reconfig(Some(&first)).await.unwrap();
    assert!(!engine.degraded());
    assert_eq!(
        engine.components.calls,
        [Part::Host, Part::Adjacencies, Part::Packets, Part::Dns].map(|part| (part, reference))
    );
    engine.components.calls.clear();
    engine.reconfig(Some(&first)).await.unwrap();
    assert!(
        engine.components.calls.is_empty(),
        "healthy renewal is a no-op"
    );
}
