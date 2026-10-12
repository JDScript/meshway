//! Trusted DNS association tasks: an exit completes its standing task once
//! per answered query; both Nodes receive a signed overlay for their own
//! netmap, revalidated whenever either netmap changes.
use super::*;
use crate::error::Error;
use meshway_configuration::execution::ExecutionAddress;
use meshway_configuration::netmap::{AssociationRecordType, DnsPlan};
use meshway_protocol::{DnsAssociationReport, SyncRequest};

async fn configure_split(lab: &Lab, rule: bool) {
    let [a, b, _] = lab.nodes;
    let current = lab
        .app
        .configurations()
        .read(&lab.owner, lab.network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    config.routing.egress.rules.clear();
    let rules = if rule {
        vec![EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                domain_suffix: vec!["chat.example".into()],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        }]
    } else {
        vec![EgressRule {
            remark: String::new(),
            matcher: RouteMatch {
                ip_cidr: vec!["8.8.8.8/32".into()],
                ..Default::default()
            },
            action: RuleAction::Exit {
                nodes: vec![format!("node:{b}")],
                via: vec![],
            },
        }]
    };
    config.routing.egress.profiles.insert(
        "split".into(),
        RoutingProfile {
            rules,
            fallback: RouteAction::Direct,
        },
    );
    config.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{a}")],
        allowed: vec!["split".into()],
        default: "split".into(),
        locked: false,
    }];
    lab.app
        .configurations()
        .apply(
            &lab.owner,
            lab.network,
            meshway_protocol::configuration::ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(canonical_json(&config).unwrap().expose().into()),
            },
            rid(),
        )
        .await
        .unwrap();
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
}

#[tokio::test]
async fn an_exit_completes_its_task_once_and_both_nodes_receive_the_overlay() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = lab(true).await;
    let [a, b, _] = lab.nodes;
    configure_split(&lab, true).await;
    let (na, nb) = (lab.netmap(a).await, lab.netmap(b).await);
    let DnsPlan::ViaExit { service } = &na.netmap.dns.egress else {
        panic!("{:?}", na.netmap.dns.egress)
    };
    assert!(matches!(
        nb.netmap.dns.egress,
        DnsPlan::ServeExitResolver { .. }
    ));
    let report = |name: &str, address: &str| DnsAssociationReport {
        netmap: nb.reference(),
        report_id: rid(),
        source: a,
        generation: service.generation,
        name: name.into(),
        record_type: AssociationRecordType::A,
        cnames: vec!["edge.cdn.example".into()],
        addresses: vec![ExecutionAddress(address.parse().unwrap())],
        ttl_seconds: 60,
        resolved_at_ms: now_ms(),
    };
    let netmaps = lab.app.netmaps();
    let first = report("api.chat.example", "104.16.0.9");
    let accepted = netmaps
        .report_association(&lab.tokens[1], first.clone())
        .await
        .unwrap();
    assert!(
        accepted.expires_at_ms <= now_ms() + 60_000 && accepted.expires_at_ms > now_ms() + 50_000
    );
    // Single use.
    assert_eq!(
        netmaps.report_association(&lab.tokens[1], first).await,
        Err(Error::RequestConflict)
    );
    // Only the exit the source resolves through, only names a name rule decides.
    let mut own = report("api.chat.example", "104.16.0.9");
    own.netmap = na.reference();
    assert_eq!(
        netmaps.report_association(&lab.tokens[0], own).await,
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        netmaps
            .report_association(&lab.tokens[1], report("example.org", "104.16.0.9"))
            .await,
        Err(Error::PermissionDenied)
    );
    let mut stale = report("api.chat.example", "104.16.0.9");
    stale.netmap.serial = stale.netmap.serial.checked_next().unwrap();
    assert_eq!(
        netmaps.report_association(&lab.tokens[1], stale).await,
        Err(Error::StateScopeChanged)
    );
    let mut other = report("api.chat.example", "104.16.0.9");
    other.generation = Digest::from_bytes([3; 32]);
    assert_eq!(
        netmaps.report_association(&lab.tokens[1], other).await,
        Err(Error::StateScopeChanged)
    );
    let mut late = report("api.chat.example", "104.16.0.9");
    late.resolved_at_ms -= 60_000;
    assert_eq!(
        netmaps.report_association(&lab.tokens[1], late).await,
        Err(Error::InvalidRequest)
    );
    assert_eq!(
        netmaps
            .report_association(&lab.tokens[1], report("lan.chat.example", "10.0.0.1"))
            .await,
        Err(Error::InvalidRequest)
    );
    // Entries every overlay would refuse are refused before acceptance, so
    // they never reach (and break) either Node's sync.
    let label = "a".repeat(63);
    let long = [label.as_str(); 4].join(".") + ".chat.example";
    assert!(long.len() > 253);
    for cnames in [vec!["é".to_owned()], vec![long.clone()]] {
        let mut bad = report("api.chat.example", "104.16.0.9");
        bad.cnames = cnames;
        assert_eq!(
            netmaps.report_association(&lab.tokens[1], bad).await,
            Err(Error::InvalidRequest)
        );
    }
    assert_eq!(
        netmaps
            .report_association(&lab.tokens[1], report(&long, "104.16.0.9"))
            .await,
        Err(Error::InvalidRequest)
    );

    let sync = |token: String, netmap, state, associations| {
        let netmaps = lab.app.netmaps();
        async move {
            netmaps
                .sync(
                    &token,
                    SyncRequest {
                        netmap: Some(netmap),
                        state,
                        server_key_generation: None,
                        traversal_requests: Default::default(),
                        associations,
                    },
                )
                .await
                .unwrap()
        }
    };
    let answer = sync(lab.tokens[0].clone(), na.reference(), None, None).await;
    let update = answer.associations.expect("overlay for the source");
    assert_eq!(update.overlay.netmap, na.reference());
    assert_eq!(update.signature.key_id, answer.signature.key_id);
    let entries = &update.overlay.associations;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        (
            entries[0].source,
            entries[0].name.as_str(),
            &entries[0].cnames[..]
        ),
        (a, "api.chat.example", &["edge.cdn.example".to_owned()][..])
    );
    // The exit holds the same association for its own netmap.
    let exit = sync(lab.tokens[1].clone(), nb.reference(), None, None).await;
    let exit = exit.associations.unwrap();
    assert_eq!(exit.overlay.netmap, nb.reference());
    assert_eq!(exit.overlay.associations, update.overlay.associations);
    // Holding it, the source is not answered again for it.
    let digest = update.overlay.digest().unwrap();
    let held = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        sync(
            lab.tokens[0].clone(),
            na.reference(),
            answer.peer_facts.as_ref().map(|facts| facts.cursor),
            Some(digest),
        ),
    )
    .await;
    assert!(held.is_err(), "an unchanged overlay holds the sync");

    // The name rule is gone: the association is revalidated away and the
    // source's next answer clears it.
    configure_split(&lab, false).await;
    let na2 = lab.netmap(a).await;
    assert_ne!(na2.reference(), na.reference());
    let cleared = sync(lab.tokens[0].clone(), na.reference(), None, Some(digest)).await;
    let cleared = cleared.associations.expect("explicit clear");
    assert!(cleared.overlay.associations.is_empty());
    assert_eq!(cleared.overlay.netmap, na2.reference());
    lab.app.shutdown().await.unwrap();
}
