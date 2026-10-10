use super::*;

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
    let created = device.create(&c, &s).await;
    assert_eq!(created.return_method, EnrollmentReturnMethod::Poll);
    // No Console origin configured: the Server hands the device no link.
    assert_eq!(created.authorization_url, None);
    let inspect: DeviceAuthorizationView = value(
        c.get(device.user_url(&s, ""))
            .bearer_auth(stranger.token.expose()),
    )
    .await;
    assert!(inspect.approval.is_none());
    reject(
        c.post(device.user_url(&s, "/deny"))
            .bearer_auth(member.token.expose()),
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
    reject(
        c.post(device.user_url(&s, "/deny"))
            .bearer_auth(member.token.expose()),
        StatusCode::CONFLICT,
        "state_conflict",
    )
    .await;
    reject(
        c.post(device.device_url(&s, "cancel"))
            .json(&device.proof("cancel")),
        StatusCode::CONFLICT,
        "state_conflict",
    )
    .await;
    let node = enrolled.node_id;
    let settings_path = format!("nodes/{node}/settings");
    let settings: NodeSettingsResponse = value(
        c.get(net(&s, n, &settings_path))
            .bearer_auth(member.token.expose()),
    )
    .await;
    assert_eq!(settings.network_id, n);
    assert_eq!(settings.node_id, node);
    assert_eq!(settings.settings.hostname, device.intent.hostname);
    reject(
        c.get(net(&s, n, "configuration"))
            .bearer_auth(member.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    reject(
        c.get(net(&s, n, "configuration/revisions"))
            .bearer_auth(member.token.expose()),
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
    reject(
        c.get(net(&s, n, &settings_path))
            .bearer_auth(member.token.expose()),
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
        reject(
            c.post(device.user_url(&s, "/approve"))
                .bearer_auth(member.token.expose())
                .json(&ApproveEnrollmentRequest { network_id: n }),
            StatusCode::CONFLICT,
            "state_conflict",
        )
        .await;
    }
    s.close().await;
}

#[tokio::test]
async fn device_authorization_links_come_from_the_configured_console_origin() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(dir.path().canonicalize().unwrap().join("controller"));
    settings.console_origin =
        Some(meshway_server::console_origin("https://console.example.test:8443").unwrap());
    let s = Server::start_with(settings).await;
    let c = client();
    let owner = register(&c, &s, "link-owner@example.test", "valid password").await;
    let n = create(&c, &s, owner.token.expose(), "device links")
        .await
        .network
        .id;
    let device = Device::new(43);
    let expected = format!(
        "https://console.example.test:8443/authorize-device/{}",
        device.intent.id
    );
    let created = device.create(&c, &s).await;
    assert_eq!(
        created.authorization_url.as_deref(),
        Some(expected.as_str())
    );
    // The same link on every view of the request: device poll, user inspect
    // and approval. Idempotent creation returns it again.
    assert_eq!(
        device.create(&c, &s).await.authorization_url.as_deref(),
        Some(expected.as_str())
    );
    let poll: EnrollmentPollResponse = value(
        c.post(device.device_url(&s, "query"))
            .json(&device.proof("poll")),
    )
    .await;
    assert_eq!(
        poll.request.authorization_url.as_deref(),
        Some(expected.as_str())
    );
    let inspect: DeviceAuthorizationView = value(
        c.get(device.user_url(&s, ""))
            .bearer_auth(owner.token.expose()),
    )
    .await;
    assert_eq!(
        inspect.request.authorization_url.as_deref(),
        Some(expected.as_str())
    );
    let approved: DeviceAuthorizationView = value(
        c.post(device.user_url(&s, "/approve"))
            .bearer_auth(owner.token.expose())
            .json(&ApproveEnrollmentRequest { network_id: n }),
    )
    .await;
    assert_eq!(
        approved.request.authorization_url.as_deref(),
        Some(expected.as_str())
    );
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
    assert_eq!(first.total.get(), 1001);
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
    assert_eq!(revoked.total.get(), 1002);
    assert!(!revoked.items[0].actions.revoke);
    assert!(revoked.items[0].revoked_at_ms.is_some());
    let second: EnrollmentKeysResponse = value(
        c.get(net(&s, n, &format!("enrollment-keys?cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
    )
    .await;
    assert_eq!(second.total.get(), 1001);
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
    reject(
        c.get(net(&s, n, &format!("enrollment-keys?cursor={cursor}")))
            .bearer_auth(admin.token.expose()),
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    // Creation is idempotent per request ID: same input replays the metadata
    // without the secret; different input is a request ID conflict, not a
    // binding problem.
    let replay_id = request();
    let replay_expiry = (chrono::Utc::now().timestamp_millis() + 600_000) as u64;
    let creation = |hostname: &str| CreateEnrollmentKeyRequest {
        request_id: replay_id,
        tags: vec!["tag:service".into()],
        initial_settings: InitialNodeSettings {
            hostname: Some(hostname.into()),
            listeners: Default::default(),
        },
        expires_at_ms: replay_expiry,
    };
    let created: EnrollmentKeyResponse = value(
        c.post(net(&s, n, "enrollment-keys"))
            .bearer_auth(owner.token.expose())
            .json(&creation("replayed")),
    )
    .await;
    assert!(created.secret.is_some());
    let replayed: EnrollmentKeyResponse = value(
        c.post(net(&s, n, "enrollment-keys"))
            .bearer_auth(owner.token.expose())
            .json(&creation("replayed")),
    )
    .await;
    assert_eq!(replayed.key.id, created.key.id);
    assert!(replayed.secret.is_none());
    reject(
        c.post(net(&s, n, "enrollment-keys"))
            .bearer_auth(owner.token.expose())
            .json(&creation("different")),
        StatusCode::CONFLICT,
        "request_id_conflict",
    )
    .await;
    s.close().await;
}
