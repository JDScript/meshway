//! Routing profile selection through the Node endpoint and the user endpoint:
//! one commit, the assignment's allowed set and lock, the persistent
//! selection under the current identity, the activity record and the netmap
//! the Node runs.
use super::*;
use crate::state::facts::NetmapReader;
use meshway_configuration::selection::{AutoBasis, ProfileKind, SelectionUnavailable};
use meshway_protocol::history::{ActivityAction, ActivityActor, ActivityTarget};

struct Selection {
    _dir: tempfile::TempDir,
    app: App,
    network: NetworkId,
    owner: String,
    member: String,
    /// Tag Node with `mesh`, `work`, `auto` and `global:<b>` allowed.
    a: NodeId,
    /// Tag exit Node.
    b: NodeId,
    /// The member's own User-identity device, `mesh` and `work` allowed.
    m: NodeId,
    token: String,
    instance: InstanceRequest,
}

async fn login(app: &App, email: &str) -> String {
    app.accounts()
        .login(
            email.into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap()
        .token
        .expose()
        .to_owned()
}

async fn configure(
    app: &App,
    owner: &str,
    network: NetworkId,
    edit: impl FnOnce(&mut NetworkConfiguration),
) {
    let current = app
        .configurations()
        .read(owner, network, None)
        .await
        .unwrap();
    let mut config = current.document.configuration.clone();
    edit(&mut config);
    app.configurations()
        .apply(
            owner,
            network,
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

fn assignment(target: NodeId, allowed: &[&str], locked: bool) -> ProfileAssignment {
    ProfileAssignment {
        target: vec![format!("node:{target}")],
        allowed: allowed.iter().map(|p| (*p).to_owned()).collect(),
        default: "mesh".into(),
        locked,
    }
}

async fn selection() -> Selection {
    let (dir, app, network, a, b, ak, _, asigner, _) = fixture().await;
    let owner = login(&app, "owner@example.com").await;
    let member = app
        .accounts()
        .register(
            "member@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let link = app
        .networks()
        .create_join_link(&owner, network, MembershipRole::Member, 600, rid())
        .await
        .unwrap();
    app.networks()
        .redeem_join_link(member.token.expose(), link.secret.unwrap().expose(), rid())
        .await
        .unwrap();
    let msigner = SigningKey::from_bytes(&[91; 32]);
    let m = enroll_user_node(&app, member.token.expose(), network, &msigner, [92; 32])
        .await
        .node_id;
    configure(&app, &owner, network, |config| {
        config.nodes.get_mut(&b).unwrap().exit_enabled = true;
        config.access = vec![AccessRule {
            remark: String::new(),
            src: vec!["*".into()],
            dst: vec!["autogroup:internet".into()],
            ip: vec!["tcp:443".into()],
            bidirectional: false,
        }];
        config.routing.egress.profiles = BTreeMap::from([(
            "work".into(),
            RoutingProfile {
                rules: vec![EgressRule {
                    remark: String::new(),
                    matcher: RouteMatch {
                        ip_cidr: vec!["8.8.8.8/32".into()],
                        ..Default::default()
                    },
                    action: RuleAction::Direct,
                }],
                fallback: RouteAction::Exit {
                    nodes: vec![format!("node:{b}")],
                    via: vec![],
                },
            },
        )]);
        let global = format!("global:{b}");
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "work", "auto", &global], false),
            assignment(m, &["mesh", "work"], false),
        ];
        config.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![CountryProfile {
                countries: vec!["US".into()],
                profile: "work".into(),
            }],
            fallback: "mesh".into(),
        });
    })
    .await;
    let service = app.execution();
    let session = session(&service, network, a, &ak, &asigner).await;
    let lease = service
        .acquire(
            session.token.expose(),
            AcquireInstanceRequest {
                instance_id: iid(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap();
    Selection {
        _dir: dir,
        app,
        network,
        owner,
        member: member.token.expose().to_owned(),
        a,
        b,
        m,
        token: session.token.expose().to_owned(),
        instance: InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
    }
}

impl Selection {
    async fn node_select(
        &self,
        expected: Option<u64>,
        profile: &str,
    ) -> Result<RoutingSelectionResponse, Error> {
        self.app
            .execution()
            .select_profile(
                &self.token,
                rid(),
                RoutingSelectionRequest {
                    instance: self.instance.clone(),
                    expected_version: expected.map(|v| Version::new(v).unwrap()),
                    profile: profile.into(),
                },
            )
            .await
    }
    async fn node_clear(&self, expected: Option<u64>) -> Result<RoutingSelectionResponse, Error> {
        self.app
            .execution()
            .clear_profile(
                &self.token,
                rid(),
                ClearRoutingSelectionRequest {
                    instance: self.instance.clone(),
                    expected_version: expected.map(|v| Version::new(v).unwrap()),
                },
            )
            .await
    }
    async fn user_clear(
        &self,
        token: &str,
        node: NodeId,
        request_id: RequestId,
    ) -> Result<meshway_configuration::selection::RoutingSelectionState, Error> {
        self.app
            .execution()
            .clear_user_profile(
                token,
                self.network,
                node,
                ClearNodeRoutingSelectionRequest { request_id },
            )
            .await
    }
    async fn netmap(
        &self,
        node: NodeId,
    ) -> std::sync::Arc<meshway_configuration::netmap::CompiledNetmap> {
        let netmaps = self.app.netmaps.clone();
        let network = self.network;
        self.app
            .executor
            .handle()
            .read(move |tx| netmaps.current(tx, network, node))
            .await
            .unwrap()
            .unwrap()
    }
    async fn activities(&self) -> Vec<meshway_protocol::history::ActivityEvent> {
        self.actions(ActivityAction::RoutingSelect).await
    }
    async fn actions(
        &self,
        action: ActivityAction,
    ) -> Vec<meshway_protocol::history::ActivityEvent> {
        self.app
            .history()
            .activities(&self.owner, Some(self.network), 200, None, rid())
            .await
            .unwrap()
            .items
            .into_iter()
            .filter(|event| event.action == action)
            .collect()
    }
}

#[tokio::test]
async fn auto_follows_the_declared_region_and_records_each_change_of_its_resolution() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let a = lab.a;
    let resolved = |netmap: &meshway_configuration::netmap::CompiledNetmap| {
        let state = meshway_policy::compile::netmap::routing_selection(&netmap.netmap).unwrap();
        let auto = state.auto.unwrap();
        (auto.profile, auto.basis, auto.region)
    };
    let generation = |netmap: &meshway_configuration::netmap::CompiledNetmap| {
        netmap.netmap.egress.as_ref().unwrap().generation
    };
    lab.node_select(None, "auto").await.unwrap();
    let none = lab.netmap(a).await;
    assert_eq!(
        resolved(&none),
        ("mesh".into(), AutoBasis::NoTrustedRegion, None)
    );
    let region = |value: Option<&str>| {
        let value = value.map(str::to_owned);
        move |config: &mut NetworkConfiguration| config.nodes.get_mut(&a).unwrap().region = value
    };
    configure(&lab.app, &lab.owner, lab.network, region(Some("US"))).await;
    let us = lab.netmap(a).await;
    assert_eq!(
        resolved(&us),
        ("work".into(), AutoBasis::Label, Some("US".into()))
    );
    // Neither `routing.egress` nor the selection version changed, yet the
    // selection identity did.
    assert_ne!(generation(&none), generation(&us));
    configure(&lab.app, &lab.owner, lab.network, region(Some("JP"))).await;
    let jp = lab.netmap(a).await;
    assert_eq!(
        resolved(&jp),
        ("mesh".into(), AutoBasis::NoRuleMatched, Some("JP".into()))
    );
    assert_ne!(generation(&jp), generation(&none));
    assert_ne!(generation(&jp), generation(&us));
    // An unrelated change keeps the resolution and records nothing.
    configure(&lab.app, &lab.owner, lab.network, |config| {
        config.nodes.get_mut(&a).unwrap().remark = "tokyo".into()
    })
    .await;
    assert_eq!(generation(&*lab.netmap(a).await), generation(&jp));
    let events = lab.actions(ActivityAction::RoutingAutoResolve).await;
    let profiles: Vec<_> = events
        .iter()
        .map(|event| match &event.target {
            ActivityTarget::RoutingSelection { node_id, profile } => {
                assert_eq!(*node_id, a);
                assert!(matches!(event.actor, ActivityActor::System {}));
                profile.as_deref().unwrap()
            }
            _ => panic!("routing.autoResolve without a selection target"),
        })
        .collect();
    // Newest first: JP, US, the first resolution without a region.
    assert_eq!(profiles, ["mesh", "work", "mesh"]);
    // A manual selection overrides auto and is no resolution change.
    lab.node_select(Some(1), "work").await.unwrap();
    let manual =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(
        (manual.applied_profile.as_str(), manual.auto),
        ("work", None)
    );
    assert_eq!(
        lab.actions(ActivityAction::RoutingAutoResolve).await.len(),
        3
    );
}

#[tokio::test]
async fn only_managers_declare_a_region_and_it_must_be_a_country_code() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let apply = |token: String, region: &str| {
        let app = &lab.app;
        let network = lab.network;
        let a = lab.a;
        let region = region.to_owned();
        let owner = lab.owner.clone();
        async move {
            let current = app
                .configurations()
                .read(&owner, network, None)
                .await
                .unwrap();
            let mut config = current.document.configuration.clone();
            config.nodes.get_mut(&a).unwrap().region = Some(region);
            app.configurations()
                .apply(
                    &token,
                    network,
                    meshway_protocol::configuration::ConfigurationRequest::Document {
                        expected_revision: current.revision,
                        request_id: rid(),
                        source_jsonc: WireSecret::new(
                            canonical_json(&config).unwrap().expose().into(),
                        ),
                    },
                    rid(),
                )
                .await
        }
    };
    assert!(matches!(
        apply(lab.member.clone(), "US").await,
        Err(Error::PermissionDenied)
    ));
    match apply(lab.owner.clone(), "us").await.err().unwrap() {
        Error::InvalidConfiguration(meshway_configuration::ConfigurationError::Invalid(issues)) => {
            assert!(issues.iter().any(|issue| issue.code
                == meshway_configuration::IssueCode::InvalidCountryCode
                && issue.path == format!("/nodes/{}/region", lab.a)))
        }
        other => panic!("{other:?}"),
    }
    assert!(apply(lab.owner.clone(), "US").await.is_ok());
}

#[tokio::test]
async fn a_node_selects_an_allowed_profile_and_runs_the_netmap_carrying_it() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let before = lab.netmap(lab.a).await;
    let initial = meshway_policy::compile::netmap::routing_selection(&before.netmap).unwrap();
    assert_eq!(
        (
            initial.requested_profile.as_deref(),
            initial.version,
            initial.applied_profile.as_str()
        ),
        (None, None, "mesh")
    );
    let names: Vec<_> = initial
        .profiles
        .iter()
        .map(|p| (p.name.clone(), p.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("mesh".to_owned(), ProfileKind::Mesh),
            ("work".to_owned(), ProfileKind::Custom),
            ("auto".to_owned(), ProfileKind::Auto),
            (format!("global:{}", lab.b), ProfileKind::Global),
        ]
    );
    let work = &initial.profiles[1];
    assert_eq!((work.rules, work.exits.as_slice()), (1, [lab.b].as_slice()));

    let selected = lab.node_select(None, "work").await.unwrap().selection;
    assert_eq!(selected.requested_profile.as_deref(), Some("work"));
    assert_eq!(selected.applied_profile, "work");
    assert_eq!(selected.version.map(Version::get), Some(1));
    // The new netmap carries the selection; the Node computes the same state
    // from it, and the selection identity (egress generation) changed.
    let after = lab.netmap(lab.a).await;
    assert!(after.reference().serial > before.reference().serial);
    assert_eq!(
        meshway_policy::compile::netmap::routing_selection(&after.netmap).unwrap(),
        selected
    );
    assert_ne!(
        before.netmap.egress.as_ref().unwrap().generation,
        after.netmap.egress.as_ref().unwrap().generation
    );

    // The compare-and-swap base is the netmap's version.
    assert_eq!(
        lab.node_select(None, "mesh").await.unwrap_err(),
        Error::VersionConflict
    );
    match lab
        .node_select(Some(1), "global:00000000-0000-4000-8000-000000000009")
        .await
    {
        Err(Error::RoutingProfileUnavailable(params)) => {
            assert_eq!(params.reason, SelectionUnavailable::NotAllowed);
            assert_eq!(params.allowed[..2], ["mesh".to_owned(), "work".to_owned()]);
        }
        other => panic!("{other:?}"),
    }
    // auto without a trusted region applies its configured fallback.
    let auto = lab.node_select(Some(1), "auto").await.unwrap().selection;
    assert_eq!(auto.applied_profile, "auto");
    let resolution = auto.auto.unwrap();
    assert_eq!(
        (resolution.profile.as_str(), resolution.basis),
        ("mesh", AutoBasis::NoTrustedRegion)
    );
    // Selecting the same profile again is still a switch: a new version.
    let again = lab.node_select(Some(2), "auto").await.unwrap().selection;
    assert_eq!(again.version.map(Version::get), Some(3));

    let events = lab.activities().await;
    assert_eq!(events.len(), 3);
    for event in &events {
        assert!(matches!(event.actor, ActivityActor::Node { node_id } if node_id == lab.a));
    }
    assert!(matches!(
        &events[2].target,
        ActivityTarget::RoutingSelection { node_id, profile } if *node_id == lab.a && profile.as_deref() == Some("work")
    ));
}

#[tokio::test]
async fn a_lock_or_a_narrowed_assignment_applies_the_default_and_a_restored_one_the_selection() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    lab.node_select(None, "work").await.unwrap();
    let global = format!("global:{}", lab.b);
    let a = lab.a;
    let m = lab.m;
    configure(&lab.app, &lab.owner, lab.network, |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "work", "auto", &global], true),
            assignment(m, &["mesh", "work"], false),
        ];
    })
    .await;
    let locked =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(locked.requested_profile.as_deref(), Some("work"));
    assert_eq!(
        locked.requested_unavailable,
        Some(SelectionUnavailable::Locked)
    );
    assert_eq!(locked.applied_profile, "mesh");
    match lab.node_select(Some(1), "work").await {
        Err(Error::RoutingProfileUnavailable(params)) => {
            assert_eq!(params.reason, SelectionUnavailable::Locked)
        }
        other => panic!("{other:?}"),
    }
    // Only the default is selectable while locked.
    lab.node_select(Some(1), "mesh").await.unwrap();
    lab.node_select(Some(2), "work").await.unwrap_err();

    configure(&lab.app, &lab.owner, lab.network, |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "auto"], false),
            assignment(m, &["mesh", "work"], false),
        ];
        config.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![],
            fallback: "mesh".into(),
        });
    })
    .await;
    let owner = lab.owner.clone();
    let execution = lab.app.execution();
    let user = execution
        .routing_selection(&owner, lab.network, a)
        .await
        .unwrap();
    assert_eq!(user.requested_profile.as_deref(), Some("mesh"));
    assert_eq!(user.applied_profile, "mesh");
    // A persistent selection outlives a narrowed assignment and applies again
    // once it is allowed.
    lab.app
        .execution()
        .select_user_profile(
            &owner,
            lab.network,
            a,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "auto".into(),
            },
        )
        .await
        .unwrap();
    configure(&lab.app, &lab.owner, lab.network, |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh"], false),
            assignment(m, &["mesh", "work"], false),
        ];
        config.routing.egress.auto = Input::Omitted;
    })
    .await;
    let narrowed = execution
        .routing_selection(&owner, lab.network, a)
        .await
        .unwrap();
    assert_eq!(narrowed.requested_profile.as_deref(), Some("auto"));
    assert_eq!(
        narrowed.requested_unavailable,
        Some(SelectionUnavailable::NotAllowed)
    );
    assert_eq!(narrowed.applied_profile, "mesh");
    assert!(narrowed.auto.is_none());
    configure(&lab.app, &lab.owner, lab.network, |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "auto"], false),
            assignment(m, &["mesh", "work"], false),
        ];
        config.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![],
            fallback: "mesh".into(),
        });
    })
    .await;
    let restored =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(restored.applied_profile, "auto");
    assert_eq!(restored.requested_unavailable, None);
}

#[tokio::test]
async fn members_select_their_own_devices_and_managers_any_node_as_themselves() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let execution = lab.app.execution();
    let select = |token: String, node: NodeId, request_id: RequestId, profile: &str| {
        let execution = execution.clone();
        let network = lab.network;
        let profile = profile.to_owned();
        async move {
            execution
                .select_user_profile(
                    &token,
                    network,
                    node,
                    SelectNodeRoutingProfileRequest {
                        request_id,
                        profile,
                    },
                )
                .await
        }
    };
    // A member sees and selects for their own device only; a Tag Node is not found.
    let own = execution
        .routing_selection(&lab.member, lab.network, lab.m)
        .await
        .unwrap();
    assert_eq!(own.applied_profile, "mesh");
    assert_eq!(
        execution
            .routing_selection(&lab.member, lab.network, lab.a)
            .await
            .unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        select(lab.member.clone(), lab.a, rid(), "work")
            .await
            .unwrap_err(),
        Error::NotFound
    );
    let request = rid();
    let chosen = select(lab.member.clone(), lab.m, request, "work")
        .await
        .unwrap();
    assert_eq!(
        (
            chosen.applied_profile.as_str(),
            chosen.version.map(Version::get)
        ),
        ("work", Some(1))
    );
    // A retry is the same selection; another profile under its ID is a conflict.
    let retried = select(lab.member.clone(), lab.m, request, "work")
        .await
        .unwrap();
    assert_eq!(retried, chosen);
    assert_eq!(
        select(lab.member.clone(), lab.m, request, "mesh")
            .await
            .unwrap_err(),
        Error::RequestConflict
    );
    // The manager selects for a Tag Node and for the member's device.
    let global = format!("global:{}", lab.b);
    select(lab.owner.clone(), lab.a, rid(), &global)
        .await
        .unwrap();
    select(lab.owner.clone(), lab.m, rid(), "mesh")
        .await
        .unwrap();
    let owner_id = lab
        .app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap()
        .session
        .user
        .id;
    let events = lab.activities().await;
    assert_eq!(events.len(), 3);
    assert!(
        matches!(&events[0].actor, ActivityActor::User { user_id, .. } if *user_id == owner_id)
    );
    assert!(
        matches!(&events[2].actor, ActivityActor::User { user_id, .. } if *user_id != owner_id)
    );
    assert!(matches!(
        &events[1].target,
        ActivityTarget::RoutingSelection { node_id, profile } if *node_id == lab.a && profile.as_deref() == Some(global.as_str())
    ));
    // The Node's netmap and its compare-and-swap base follow the user's selection.
    assert_eq!(
        lab.node_select(None, "mesh").await.unwrap_err(),
        Error::VersionConflict
    );
    let carried =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(lab.a).await.netmap)
            .unwrap();
    assert_eq!(carried.applied_profile, global);
    lab.node_select(Some(1), "mesh").await.unwrap();
    // The inventory summarizes each visible Node's selection.
    let inventory = execution.inventory(&lab.owner, lab.network).await.unwrap();
    let a = inventory.items.iter().find(|n| n.id == lab.a).unwrap();
    assert_eq!(
        (
            a.routing.requested_profile.as_deref(),
            a.routing.applied_profile.as_str(),
            a.routing.can_select
        ),
        (Some("mesh"), "mesh", true)
    );
    let mine = execution.inventory(&lab.member, lab.network).await.unwrap();
    assert_eq!(mine.items.len(), 1);
    assert_eq!(mine.items[0].routing.applied_profile, "mesh");
}

#[tokio::test]
async fn the_user_routes_answer_the_selection_and_refuse_a_profile_with_the_allowed_ones() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/v1/networks/{}/nodes/{}/routing-selection",
        listener.local_addr().unwrap(),
        lab.network,
        lab.m
    );
    let router = crate::http::router(&lab.app);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(meshway_transport_stream::tls::platform_client_config().unwrap())
        .build()
        .unwrap();
    let read = client
        .get(&base)
        .bearer_auth(&lab.member)
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let read: NodeRoutingSelectionResponse = read.json().await.unwrap();
    assert_eq!((read.node_id, read.can_select), (lab.m, true));
    let refused = client
        .post(&base)
        .bearer_auth(&lab.member)
        .json(&serde_json::json!({"requestId": rid(), "profile": "auto"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 422);
    let problem: meshway_protocol::ErrorResponse = refused.json().await.unwrap();
    match problem.problem {
        meshway_protocol::Problem::RoutingProfileUnavailable { params, .. } => {
            assert_eq!(params.reason, SelectionUnavailable::NotAllowed);
            assert_eq!(params.allowed, ["mesh", "work"]);
        }
        other => panic!("{other:?}"),
    }
    let selected = client
        .post(&base)
        .bearer_auth(&lab.member)
        .json(&serde_json::json!({"requestId": rid(), "profile": "work"}))
        .send()
        .await
        .unwrap();
    assert_eq!(selected.status(), 200);
    let selected: NodeRoutingSelectionResponse = selected.json().await.unwrap();
    assert_eq!(selected.selection.applied_profile, "work");
    let cleared = client
        .delete(&base)
        .bearer_auth(&lab.member)
        .json(&serde_json::json!({"requestId": rid()}))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 200);
    let cleared: NodeRoutingSelectionResponse = cleared.json().await.unwrap();
    assert_eq!(
        (
            cleared.selection.requested_profile,
            cleared.selection.applied_profile.as_str(),
            cleared.can_select
        ),
        (None, "mesh", true)
    );
    // The activity reads a cleared selection as a null profile.
    let activity = client
        .get(format!(
            "http://{}/api/v1/networks/{}/activity",
            base.split('/').nth(2).unwrap(),
            lab.network
        ))
        .bearer_auth(&lab.owner)
        .send()
        .await
        .unwrap();
    assert_eq!(activity.status(), 200);
    let activity: serde_json::Value = activity.json().await.unwrap();
    let newest = &activity["items"][0];
    assert_eq!(newest["action"], "routing.select");
    assert_eq!(newest["target"]["profile"], serde_json::Value::Null);
    assert_eq!(newest["target"]["nodeId"], lab.m.to_string());
}

#[tokio::test]
async fn a_node_clears_its_selection_under_its_version_and_the_default_applies() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    // Nothing to clear: no change, no activity, no new netmap.
    let before = lab.netmap(lab.a).await;
    let nothing = lab.node_clear(None).await.unwrap().selection;
    assert_eq!((nothing.requested_profile, nothing.version), (None, None));
    assert_eq!(
        lab.netmap(lab.a).await.reference().serial,
        before.reference().serial
    );
    assert!(lab.activities().await.is_empty());

    lab.node_select(None, "work").await.unwrap();
    // The compare-and-swap base is the held version, as for a selection.
    assert_eq!(
        lab.node_clear(None).await.unwrap_err(),
        Error::VersionConflict
    );
    let selected = lab.netmap(lab.a).await;
    let cleared = lab.node_clear(Some(1)).await.unwrap().selection;
    assert_eq!(
        (
            cleared.requested_profile.as_deref(),
            cleared.version,
            cleared.requested_unavailable,
            cleared.applied_profile.as_str()
        ),
        (None, None, None, "mesh")
    );
    // The new netmap carries no selection version and the same state.
    let after = lab.netmap(lab.a).await;
    assert!(after.reference().serial > selected.reference().serial);
    assert!(
        !after
            .netmap
            .projection
            .selection_versions
            .contains_key(&lab.a)
    );
    assert_eq!(
        meshway_policy::compile::netmap::routing_selection(&after.netmap).unwrap(),
        cleared
    );
    // Versions keep growing: the clearing used 2.
    let again = lab.node_select(None, "auto").await.unwrap().selection;
    assert_eq!(again.version.map(Version::get), Some(3));
    let events = lab.activities().await;
    assert_eq!(events.len(), 3);
    assert!(matches!(event_profile(&events[1]), (node, None) if node == lab.a));
    assert!(matches!(events[1].actor, ActivityActor::Node { node_id } if node_id == lab.a));
    assert_eq!(event_profile(&events[0]).1.as_deref(), Some("auto"));
}

fn event_profile(event: &meshway_protocol::history::ActivityEvent) -> (NodeId, Option<String>) {
    match &event.target {
        ActivityTarget::RoutingSelection { node_id, profile } => (*node_id, profile.clone()),
        _ => panic!("routing.select without a selection target"),
    }
}

#[tokio::test]
async fn clearing_a_selection_the_assignment_no_longer_allows_restores_the_default_auto() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let (a, m, b) = (lab.a, lab.m, lab.b);
    let with_default = |allowed: &[&str], default: &str| ProfileAssignment {
        target: vec![format!("node:{a}")],
        allowed: allowed.iter().map(|p| (*p).to_owned()).collect(),
        default: default.into(),
        locked: false,
    };
    lab.node_select(None, "work").await.unwrap();
    // The administrator removes `work` and makes `auto` the default: the
    // persistent selection stays, not applied.
    let global = format!("global:{b}");
    let narrowed = with_default(&["auto", "mesh", &global], "auto");
    configure(&lab.app, &lab.owner, lab.network, move |config| {
        config.routing.egress.assignments = vec![narrowed, assignment(m, &["mesh", "work"], false)];
        config.nodes.get_mut(&a).unwrap().region = Some("US".into());
        config.routing.egress.auto = Input::Value(AutoProfile {
            rules: vec![CountryProfile {
                countries: vec!["US".into()],
                profile: format!("global:{b}"),
            }],
            fallback: "mesh".into(),
        });
    })
    .await;
    let stuck =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(stuck.requested_profile.as_deref(), Some("work"));
    assert_eq!(
        stuck.requested_unavailable,
        Some(SelectionUnavailable::NotAllowed)
    );
    assert_eq!(stuck.applied_profile, "auto");
    // The owner clears it: the selection is gone and `auto` resolves as before.
    let request = rid();
    let cleared = lab.user_clear(&lab.owner, a, request).await.unwrap();
    assert_eq!(
        (
            cleared.requested_profile.as_deref(),
            cleared.requested_unavailable,
            cleared.applied_profile.as_str()
        ),
        (None, None, "auto")
    );
    let carried =
        meshway_policy::compile::netmap::routing_selection(&lab.netmap(a).await.netmap).unwrap();
    assert_eq!(carried, cleared);
    let auto = carried.auto.unwrap();
    assert_eq!(
        (auto.profile, auto.basis),
        (format!("global:{b}"), AutoBasis::Label)
    );
    // A retry is the same clearing; its ID cannot select a profile.
    let events = lab.activities().await.len();
    assert_eq!(
        lab.user_clear(&lab.owner, a, request).await.unwrap(),
        cleared
    );
    assert_eq!(lab.activities().await.len(), events);
    assert_eq!(
        lab.app
            .execution()
            .select_user_profile(
                &lab.owner,
                lab.network,
                a,
                SelectNodeRoutingProfileRequest {
                    request_id: request,
                    profile: "mesh".into(),
                },
            )
            .await
            .unwrap_err(),
        Error::RequestConflict
    );
}

#[tokio::test]
async fn members_clear_only_their_own_devices_and_a_lock_does_not_refuse_it() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let lab = selection().await;
    let execution = lab.app.execution();
    execution
        .select_user_profile(
            &lab.owner,
            lab.network,
            lab.a,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        lab.user_clear(&lab.member, lab.a, rid()).await.unwrap_err(),
        Error::NotFound
    );
    execution
        .select_user_profile(
            &lab.member,
            lab.network,
            lab.m,
            SelectNodeRoutingProfileRequest {
                request_id: rid(),
                profile: "work".into(),
            },
        )
        .await
        .unwrap();
    let own = lab.user_clear(&lab.member, lab.m, rid()).await.unwrap();
    assert_eq!(
        (own.requested_profile, own.applied_profile.as_str()),
        (None, "mesh")
    );
    // A lock pins the default, which is what clearing leaves.
    let (a, m) = (lab.a, lab.m);
    let global = format!("global:{}", lab.b);
    configure(&lab.app, &lab.owner, lab.network, move |config| {
        config.routing.egress.assignments = vec![
            assignment(a, &["mesh", "work", "auto", &global], true),
            assignment(m, &["mesh", "work"], false),
        ];
    })
    .await;
    let locked = lab.user_clear(&lab.owner, lab.a, rid()).await.unwrap();
    assert_eq!(
        (
            locked.requested_profile,
            locked.locked,
            locked.applied_profile.as_str()
        ),
        (None, true, "mesh")
    );
    let events = lab.activities().await;
    let member = &events[1];
    assert!(matches!(&member.actor, ActivityActor::User { .. }));
    assert_eq!(event_profile(member), (lab.m, None));
    assert_eq!(event_profile(&events[0]), (lab.a, None));
}
