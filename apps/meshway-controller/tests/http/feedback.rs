use super::management::{create, join, net, request, source};
use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::*;
use meshway_model::{MembershipRole, Version};
use meshway_protocol::{configuration::*, history::*, management::*};
use meshway_protocol::{enrollment::*, node::*};
use sha2::{Digest as _, Sha256};

async fn value<T: serde::de::DeserializeOwned>(builder: reqwest::RequestBuilder) -> T {
    let response = builder.send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

struct Device {
    signer: SigningKey,
    intent: EnrollmentIntent,
    verifier: String,
}
impl Device {
    fn new(seed: u8) -> Self {
        let signer = SigningKey::from_bytes(&[seed; 32]);
        let verifier = "v".repeat(43);
        Self {
            intent: EnrollmentIntent {
                id: EnrollmentRequestId::new(*request().as_uuid()).unwrap(),
                existing: None,
                control_key: NodePublicKey {
                    algorithm: KeyAlgorithm::Ed25519,
                    bytes: signer.verifying_key().to_bytes().into(),
                },
                dataplane_key: NodePublicKey {
                    algorithm: KeyAlgorithm::X25519,
                    bytes: [seed + 40; 32].into(),
                },
                hostname: format!("device-{seed}"),
                code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            },
            signer,
            verifier,
        }
    }
    fn proof(&self, operation: &str) -> EnrollmentDeviceProof {
        let digest = meshway_controller::execution::proof::request_digest(&self.intent).unwrap();
        EnrollmentDeviceProof {
            verifier: WireSecret::new(self.verifier.clone()),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    self.signer
                        .sign(&enrollment_transcript(self.intent.id, digest, operation))
                        .to_bytes(),
                ),
            ),
        }
    }
    async fn create(&self, c: &Client, s: &Server) -> EnrollmentRequestView {
        value(
            c.post(format!("{}/agent-api/v1/enrollment-requests", s.url))
                .json(&CreateEnrollmentRequest {
                    intent: self.intent.clone(),
                    signature: self.proof("create").signature,
                }),
        )
        .await
    }
    fn user_url(&self, s: &Server, action: &str) -> String {
        format!(
            "{}/api/v1/device-authorizations/{}{}",
            s.url, self.intent.id, action
        )
    }
    fn device_url(&self, s: &Server, action: &str) -> String {
        format!(
            "{}/agent-api/v1/enrollment-requests/{}/{}",
            s.url, self.intent.id, action
        )
    }
}

#[tokio::test]
async fn signed_device_lifecycle_own_settings_and_key_management_are_distinct() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "device-owner@example.test", "valid password").await;
    let member = register(&c, &s, "device-member@example.test", "valid password").await;
    let stranger = register(&c, &s, "device-stranger@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "device views")
        .await
        .network
        .id;
    join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let device = Device::new(41);
    assert_eq!(
        device.create(&c, &s).await.return_method,
        EnrollmentReturnMethod::Poll
    );
    let inspect: DeviceAuthorizationView = value(
        c.get(device.user_url(&s, ""))
            .bearer_auth(stranger.token.expose()),
    )
    .await;
    assert!(inspect.approval.is_none());
    error(
        c.post(device.user_url(&s, "/deny"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let approved: DeviceAuthorizationView = value(
        c.post(device.user_url(&s, "/approve"))
            .bearer_auth(member.token.expose())
            .json(&ApproveEnrollmentRequest { network_id: n }),
    )
    .await;
    assert_eq!(approved.approval.unwrap().user_id, member.user.id);
    let hidden: DeviceAuthorizationView = value(
        c.get(device.user_url(&s, ""))
            .bearer_auth(stranger.token.expose()),
    )
    .await;
    assert!(hidden.approval.is_none());
    let poll: EnrollmentPollResponse = value(
        c.post(device.device_url(&s, "query"))
            .json(&device.proof("poll")),
    )
    .await;
    let enrolled: EnrollmentResponse = value(c.post(device.device_url(&s, "redeem")).json(
        &RedeemEnrollmentRequest {
            proof: device.proof("redeem"),
            code: poll.code.unwrap(),
        },
    ))
    .await;
    error(
        c.post(device.user_url(&s, "/deny"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "state_conflict",
    )
    .await;
    error(
        c.post(device.device_url(&s, "cancel"))
            .json(&device.proof("cancel"))
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "state_conflict",
    )
    .await;
    let node = enrolled.node_id;
    let settings_path = format!("nodes/{node}/settings");
    let settings: meshway_configuration::NodeSettingsView = value(
        c.get(net(&s, n, &settings_path))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(settings.hostname, device.intent.hostname);
    error(
        c.get(net(&s, n, "configuration"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    error(
        c.get(net(&s, n, "configuration/revisions"))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let inventory: NodeInventoryView = value(
        c.get(net(&s, n, &format!("nodes/{node}")))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert!(
        !inventory.can_delete
            && !inventory.can_revoke_control_key
            && !inventory.can_revoke_dataplane_key
    );
    assert!(inventory.control_key_created_at_ms >= inventory.registered_at_ms);
    let current = source(&c, &s, n, owner.token.expose()).await;
    let applied: ConfigurationAppliedResponse = value(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: request(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: [("tag:service".into(), vec![member.user.email.clone()])]
                            .into(),
                    },
                    meshway_configuration::ConfigurationOperation::SetNodeSettings {
                        node_id: node,
                        fields: meshway_configuration::NodeSettingsPatch {
                            remark: meshway_configuration::Input::Value("new target".into()),
                            ..Default::default()
                        },
                    },
                ],
            }),
    )
    .await;
    assert_eq!(applied.target_count, 1);
    let progress: meshway_protocol::execution::PublicationProgress = value(
        c.get(net(
            &s,
            n,
            &format!("publications/{}", applied.publication_ids[0]),
        ))
        .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(
        progress.nodes[&node].freshness,
        meshway_protocol::execution::ReceiptFreshness::AwaitingInstance
    );
    let tagged: ConfigurationAppliedResponse = value(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Operations {
                expected_revision: applied.revision,
                request_id: request(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::SetIdentityTags {
                        node_id: node,
                        tags: vec!["tag:service".into()],
                    },
                ],
            }),
    )
    .await;
    assert!(tagged.revision > applied.revision);
    error(
        c.get(net(&s, n, &settings_path))
            .bearer_auth(member.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let revoke = |token: &str, generation| {
        c.post(net(&s, n, &format!("nodes/{node}/keys/revoke")))
            .bearer_auth(token)
            .json(&RevokeNodeKeyRequest {
                key_id: enrolled.keys.control.id,
                expected_generation: generation,
            })
    };
    error(
        revoke(member.token.expose(), Version::MIN)
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    error(
        revoke(owner.token.expose(), Version::new(2).unwrap())
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "version_conflict",
    )
    .await;
    revoke(owner.token.expose(), Version::MIN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let inventory: NodeInventoryView = value(
        c.get(net(&s, n, &format!("nodes/{node}")))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert!(
        inventory.can_delete
            && !inventory.can_revoke_control_key
            && inventory.can_revoke_dataplane_key
    );
    for (seed, withdraw) in [(42, false), (43, true)] {
        let device = Device::new(seed);
        device.create(&c, &s).await;
        if withdraw {
            let _: DeviceAuthorizationView = value(
                c.post(device.user_url(&s, "/approve"))
                    .bearer_auth(member.token.expose())
                    .json(&ApproveEnrollmentRequest { network_id: n }),
            )
            .await;
            c.post(device.user_url(&s, "/deny"))
                .bearer_auth(member.token.expose())
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        } else {
            c.post(device.device_url(&s, "cancel"))
                .json(&device.proof("cancel"))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }
        let poll: EnrollmentPollResponse = value(
            c.post(device.device_url(&s, "query"))
                .json(&device.proof("poll")),
        )
        .await;
        assert_eq!(
            poll.request.phase,
            if withdraw {
                EnrollmentPhase::Withdrawn
            } else {
                EnrollmentPhase::Cancelled
            }
        );
        assert!(poll.code.is_none());
        error(
            c.post(device.user_url(&s, "/approve"))
                .bearer_auth(member.token.expose())
                .json(&ApproveEnrollmentRequest { network_id: n })
                .send()
                .await
                .unwrap(),
            StatusCode::CONFLICT,
            "state_conflict",
        )
        .await;
    }
    s.close().await;
}

#[tokio::test]
async fn enrollment_keys_have_bounded_pages_safe_summaries_and_current_permissions() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "keys-owner@example.test", "valid password").await;
    let admin = register(&c, &s, "keys-admin@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "key pages")
        .await
        .network
        .id;
    let admin_id = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let current = source(&c, &s, n, owner.token.expose()).await;
    let _: ConfigurationAppliedResponse = value(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: request(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::ReplaceTagsOwners {
                        tags_owners: [("tag:service".into(), vec![])].into(),
                    },
                ],
            }),
    )
    .await;
    let mut last = None;
    for i in 0..1001 {
        let issued = s
            .app
            .enrollment()
            .create_key(
                owner.token.expose(),
                n,
                CreateEnrollmentKeyRequest {
                    request_id: request(),
                    tags: vec!["tag:service".into()],
                    initial_settings: InitialNodeSettings {
                        hostname: Some(format!("key-{i}")),
                        listeners: if i == 1000 { serde_json::from_str(r#"{"vless":{"type":"vless","listenPort":24443,"stream":"websocket","uuid":"11111111-1111-4111-8111-111111111111","tls":{"mode":"managed","serverName":"node.example.test"},"websocket":{"path":"/mesh","responseHeaders":{"X-Test":"private header value"}}}}"#).unwrap() } else { Default::default() },
                    },
                    expires_at_ms: (chrono::Utc::now().timestamp_millis() + 600_000) as u64,
                },
            )
            .await
            .unwrap();
        last = Some(issued);
    }
    let issued = last.unwrap();
    let first: EnrollmentKeysResponse = value(
        c.get(net(&s, n, "enrollment-keys?limit=200"))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(first.total, 1001);
    assert_eq!(first.items.len(), 200);
    assert_eq!(first.items[0].id, issued.key.id);
    assert_eq!(first.items[0].created_by, owner.user.id);
    assert_eq!(
        first.items[0].initial_settings.hostname.as_deref(),
        Some("key-1000")
    );
    assert!(first.items[0].actions.revoke);
    let encoded = serde_json::to_string(&first).unwrap();
    assert!(!encoded.contains("private header value"));
    assert!(!encoded.contains("11111111-1111-4111-8111-111111111111"));
    assert!(!encoded.contains("PRIVATE KEY"));
    assert!(
        !first.items[0].initial_settings.listeners["vless"]
            .tls
            .as_ref()
            .unwrap()
            .private_key_present
    );
    let cursor = first.next_cursor.unwrap();
    let device = Device::new(44);
    let enrolled: EnrollmentResponse = value(
        c.post(format!("{}/agent-api/v1/enrollments", s.url))
            .json(&KeyEnrollmentRequest {
                enrollment_key: issued.secret.unwrap(),
                verifier: WireSecret::new(device.verifier.clone()),
                request: CreateEnrollmentRequest {
                    intent: device.intent.clone(),
                    signature: device.proof("create").signature,
                },
            }),
    )
    .await;
    let used: EnrollmentKeysResponse = value(
        c.get(net(&s, n, "enrollment-keys?limit=1"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(used.items[0].node_id, Some(enrolled.node_id));
    assert!(used.items[0].consumed_at_ms.is_some());
    assert!(!used.items[0].actions.revoke);
    let newer = s
        .app
        .enrollment()
        .create_key(
            owner.token.expose(),
            n,
            CreateEnrollmentKeyRequest {
                request_id: request(),
                tags: vec!["tag:service".into()],
                initial_settings: InitialNodeSettings {
                    hostname: None,
                    listeners: Default::default(),
                },
                expires_at_ms: (chrono::Utc::now().timestamp_millis() + 600_000) as u64,
            },
        )
        .await
        .unwrap();
    c.delete(net(&s, n, &format!("enrollment-keys/{}", newer.key.id)))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let revoked: EnrollmentKeysResponse = value(
        c.get(net(&s, n, "enrollment-keys?limit=1"))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(revoked.total, 1002);
    assert!(!revoked.items[0].actions.revoke);
    assert!(revoked.items[0].revoked_at_ms.is_some());
    let second: EnrollmentKeysResponse = value(
        c.get(net(&s, n, &format!("enrollment-keys?cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(second.total, 1001);
    assert!(second.items.iter().all(|item| item.id != issued.key.id));
    let _: MemberMutationResponse = value(
        c.post(net(&s, n, &format!("members/{admin_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: Version::MIN,
                role: MembershipRole::Member,
            }),
    )
    .await;
    error(
        c.get(net(&s, n, &format!("enrollment-keys?cursor={cursor}")))
            .bearer_auth(admin.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    s.close().await;
}

#[tokio::test]
async fn join_link_actions_pagination_inspection_and_issuer_permission_loss() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "links-owner@example.test", "valid password").await;
    let admin = register(&c, &s, "links-admin@example.test", "valid password").await;
    let member = register(&c, &s, "links-member@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "links")
        .await
        .network
        .id;
    let admin_id = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    for (user, expected) in [
        (
            &owner,
            vec![
                MembershipRole::Owner,
                MembershipRole::Admin,
                MembershipRole::Member,
            ],
        ),
        (&admin, vec![MembershipRole::Member]),
    ] {
        let network: meshway_protocol::NetworkResponse = value(
            c.get(net(&s, n, "").trim_end_matches('/'))
                .bearer_auth(user.token.expose()),
        )
        .await;
        assert_eq!(network.network.actions.join_link_roles, expected);
    }
    let make = |token: &str, role| {
        c.post(net(&s, n, "join-links"))
            .bearer_auth(token)
            .json(&CreateJoinLinkRequest {
                request_id: request(),
                role,
                expires_in_seconds: 600,
            })
    };
    error(
        make(admin.token.expose(), MembershipRole::Owner)
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let owner_link: JoinLinkCreated =
        value(make(owner.token.expose(), MembershipRole::Owner)).await;
    let JoinLinkCreated::Available {
        link: owner_link,
        secret: owner_secret,
        ..
    } = owner_link
    else {
        panic!()
    };
    let admin_link: JoinLinkCreated =
        value(make(admin.token.expose(), MembershipRole::Member)).await;
    let JoinLinkCreated::Available {
        link: admin_link,
        secret: admin_secret,
        ..
    } = admin_link
    else {
        panic!()
    };
    assert!(owner_link.actions.revoke && admin_link.actions.revoke);
    let inspect = |token: &str, secret: &WireSecret| {
        c.post(format!("{}/api/v1/join-links/inspect", s.url))
            .bearer_auth(token)
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            })
    };
    let inspection: JoinLinkInspection = value(inspect(owner.token.expose(), &owner_secret)).await;
    assert!(inspection.already_member);
    let first: JoinLinkListResponse = value(
        c.get(net(&s, n, "join-links?limit=1"))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(first.total, 3);
    assert_eq!(first.items[0].id, admin_link.id);
    let cursor = first.next_cursor.unwrap();
    let _: JoinLinkCreated = value(make(owner.token.expose(), MembershipRole::Member)).await;
    let second: JoinLinkListResponse = value(
        c.get(net(&s, n, &format!("join-links?limit=1&cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(second.total, 3);
    assert_eq!(second.items[0].id, owner_link.id);
    assert!(!second.items[0].actions.revoke);
    error(
        c.post(net(&s, n, &format!("join-links/{}/revoke", owner_link.id)))
            .bearer_auth(admin.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    let _: MemberMutationResponse = value(
        c.post(net(&s, n, &format!("members/{admin_id}/role")))
            .bearer_auth(owner.token.expose())
            .json(&ChangeMemberRoleRequest {
                expected_version: Version::MIN,
                role: MembershipRole::Member,
            }),
    )
    .await;
    error(
        inspect(member.token.expose(), &admin_secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_issuer_unavailable",
    )
    .await;
    error(
        c.get(net(&s, n, &format!("join-links?cursor={cursor}")))
            .bearer_auth(admin.token.expose())
            .send()
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    c.post(net(&s, n, &format!("join-links/{}/revoke", owner_link.id)))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    error(
        inspect(member.token.expose(), &owner_secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_revoked",
    )
    .await;
    let consumed: JoinLinkCreated = value(make(owner.token.expose(), MembershipRole::Member)).await;
    let JoinLinkCreated::Available { secret, .. } = consumed else {
        panic!()
    };
    let redeem = || {
        c.post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(member.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            })
    };
    let joined: JoinLinkRedeemed = value(redeem()).await;
    let retry: JoinLinkRedeemed = value(redeem()).await;
    assert_eq!(joined.membership_id, retry.membership_id);
    error(
        inspect(member.token.expose(), &secret)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_used",
    )
    .await;
    let revision = source(&c, &s, n, owner.token.expose()).await.revision;
    c.post(net(
        &s,
        n,
        &format!("members/{}/remove", joined.membership_id),
    ))
    .bearer_auth(owner.token.expose())
    .json(&RemoveMemberRequest {
        expected_version: Version::MIN,
        expected_revision: revision,
    })
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    error(
        redeem().send().await.unwrap(),
        StatusCode::BAD_REQUEST,
        "join_link_used",
    )
    .await;
    s.close().await;
}

#[tokio::test]
async fn defaults_replay_preserves_other_admin_changes_and_reports_precise_conflicts() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "defaults-owner@example.test", "valid password").await;
    let admin = register(&c, &s, "defaults-admin@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "partial defaults")
        .await
        .network
        .id;
    let member = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        admin.token.expose(),
        MembershipRole::Admin,
    )
    .await;
    let initial = source(&c, &s, n, owner.token.expose()).await;
    let op: meshway_configuration::ConfigurationOperation =
        serde_json::from_str(r#"{"type":"replaceDefaults","defaults":{"topology":"deny"}}"#)
            .unwrap();
    let mut concurrent =
        meshway_configuration::NetworkConfigurationInput::from(&initial.configuration);
    concurrent.defaults.dialers.vless.as_mut().unwrap().priority = 19;
    concurrent
        .defaults
        .dialers
        .vless
        .as_mut()
        .unwrap()
        .websocket = Some(
        serde_json::from_str(r#"{"headers":{"Authorization":"private concurrent secret"}}"#)
            .unwrap(),
    );
    let first: ConfigurationAppliedResponse = c
        .post(net(&s, n, "configuration"))
        .bearer_auth(admin.token.expose())
        .json(&ConfigurationRequest::Document {
            expected_revision: initial.revision,
            request_id: request(),
            source_jsonc: WireSecret::new(serde_json::to_string(&concurrent).unwrap()),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first.target_count, 0);
    assert!(first.publication_ids.is_empty());
    let rid = request();
    let stale = ConfigurationRequest::Operations {
        expected_revision: initial.revision,
        request_id: rid,
        ops: vec![op.clone()],
    };
    error(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&stale)
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    let replay = ConfigurationRequest::Operations {
        expected_revision: first.revision,
        request_id: rid,
        ops: vec![op],
    };
    let applied: ConfigurationAppliedResponse = c
        .post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&replay)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let saved = source(&c, &s, n, owner.token.expose()).await;
    assert_eq!(
        saved.configuration.defaults.dialers,
        concurrent.defaults.dialers
    );
    assert_eq!(
        saved.configuration.defaults.access,
        initial.configuration.defaults.access
    );
    assert_eq!(
        saved.configuration.defaults.topology,
        meshway_configuration::TopologyDefault::Deny
    );
    let different = ConfigurationRequest::Operations {
        expected_revision: first.revision,
        request_id: rid,
        ops: vec![],
    };
    error(
        c.post(net(&s, n, "configuration"))
            .bearer_auth(owner.token.expose())
            .json(&different)
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "request_id_conflict",
    )
    .await;
    error(
        c.post(net(&s, n, &format!("members/{member}/remove")))
            .bearer_auth(owner.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: Version::MIN,
                expected_revision: initial.revision,
            })
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await;
    error(
        c.post(net(&s, n, &format!("members/{member}/remove")))
            .bearer_auth(owner.token.expose())
            .json(&RemoveMemberRequest {
                expected_version: Version::new(2).unwrap(),
                expected_revision: applied.revision,
            })
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "version_conflict",
    )
    .await;
    let clear: meshway_configuration::ConfigurationOperation =
        serde_json::from_str(r#"{"type":"replaceDefaults","defaults":{"dialers":{}}}"#).unwrap();
    c.post(net(&s, n, "configuration"))
        .bearer_auth(owner.token.expose())
        .json(&ConfigurationRequest::Operations {
            expected_revision: applied.revision,
            request_id: request(),
            ops: vec![clear],
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(
        source(&c, &s, n, owner.token.expose())
            .await
            .configuration
            .defaults
            .dialers
            .vless
            .is_none()
    );
    s.close().await;
}

#[tokio::test]
async fn historical_actor_email_is_a_snapshot_after_departure_and_email_change() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "history-owner@example.test", "valid password").await;
    let member = register(&c, &s, "history-old@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "history snapshots")
        .await
        .network
        .id;
    let membership = join(
        &c,
        &s,
        n,
        owner.token.expose(),
        member.token.expose(),
        MembershipRole::Member,
    )
    .await;
    let revision = source(&c, &s, n, owner.token.expose()).await.revision;
    c.post(net(&s, n, &format!("members/{membership}/remove")))
        .bearer_auth(owner.token.expose())
        .json(&RemoveMemberRequest {
            expected_version: Version::MIN,
            expected_revision: revision,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    c.post(format!("{}/api/v1/account/email", s.url))
        .bearer_auth(member.token.expose())
        .json(&ChangeEmailRequest {
            password: WireSecret::new("valid password".into()),
            email: "history-private-new@example.test".into(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let text = c
        .get(net(&s, n, "activity"))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("history-private-new"));
    let history: ActivityListResponse = serde_json::from_str(&text).unwrap();
    assert!(history.items.iter().any(|event| matches!(&event.actor, ActivityActor::User { user_id, email } if *user_id == member.user.id && email.as_deref() == Some("history-old@example.test"))));
    let unknown: ActivityActor = serde_json::from_value(
        serde_json::json!({"type":"user","userId":member.user.id,"email":null}),
    )
    .unwrap();
    assert!(matches!(unknown, ActivityActor::User { email: None, .. }));
    s.close().await;
}

#[tokio::test]
async fn transport_catalog_references_configuration_schema_definitions() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Server::start(&dir.path().canonicalize().unwrap().join("controller"), true).await;
    let c = client();
    let owner = register(&c, &s, "catalog@example.test", "valid password").await;
    let schema: ConfigurationSchemaResponse = c
        .get(format!("{}/api/v1/configuration/schema", s.url))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let catalog: TransportCatalogResponse = c
        .get(format!("{}/api/v1/transport-catalog", s.url))
        .bearer_auth(owner.token.expose())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema.schema_json).unwrap();
    for entry in catalog.items {
        for name in [entry.listener_schema, entry.dialer_schema] {
            assert!(schema["$defs"].get(&name).is_some(), "missing {name}");
        }
    }
    s.close().await;
}
