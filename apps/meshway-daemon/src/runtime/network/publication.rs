//! Real Server/Control publication ordering at the controllable worker boundary.
//! Kernel/cipher continuity is verified by the separate Linux packet fixture.
use super::*;
use crate::runtime::ping::integration::Fixture;

#[tokio::test]
async fn dns_demand_during_publish_cannot_skip_an_unoffered_business_target() {
    let mut fixture = Fixture::open().await;
    let destination = fixture.controls[1]
        .cached_target()
        .unwrap()
        .configuration
        .addresses
        .ipv4
        .unwrap();
    let control = &mut fixture.controls[0];
    let previous = control.execution_snapshot().unwrap();
    let profile = previous.configuration().unwrap().dns.profile.clone();
    control
        .create_demand(&meshway_protocol::DemandRequest {
            instance: previous.instance(),
            request_id: packets::request_id().unwrap(),
            target: meshway_configuration::execution::BusinessTarget::Ip {
                address: meshway_configuration::execution::ExecutionAddress(destination.into()),
            },
            protocol: meshway_configuration::execution::BusinessProtocol::IcmpEcho,
            port: None,
            profile: profile.clone(),
            duration_seconds: 120,
        })
        .await
        .unwrap();
    control.refresh_configuration().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    let business = control.execution_snapshot().unwrap();
    assert_ne!(business.reference(), previous.reference());
    assert!(business.continuation_from(&previous).is_ok());

    let plan = dns::intents::Plan::new(
        business.instance(),
        business.configuration().unwrap(),
        vec![meshway_dns::ResolverDemand {
            source: meshway_dns::ResolverSource::Configured("ordering-test".into()),
            endpoint: "192.0.2.53:53".parse().unwrap(),
            transport: meshway_configuration::DnsTransport::Udp,
            profile,
        }],
    );
    let (_plans, received) = watch::channel(Some(plan));
    let mut worker = testing::Worker::new();
    worker.handle.dns_plans = received;
    worker.handle.publish(control).await.unwrap();
    let offered = worker
        .handle
        .target
        .borrow()
        .as_ref()
        .unwrap()
        .identity
        .as_ref()
        .unwrap()
        .snapshot()
        .clone();
    assert_eq!(offered.reference(), business.reference());
    assert!(offered.continuation_from(&previous).is_ok());
    business.ensure_current().unwrap();

    // The new DNS demand has a real Server decision, but no unobserved target
    // may replace the business target within publish. The next cycle observes
    // it normally; even a denied resolver must not erase an intermediate scope.
    control.refresh_configuration().await.unwrap();
    let next = control.execution_snapshot().unwrap();
    assert_ne!(next.reference(), business.reference());
    assert!(next.continuation_from(&offered).is_ok());
    assert!(next.continuation_from(&previous).is_err());
    worker.close().await;
    fixture.close().await;
}
