//! Catalog search, member listing and match queries over verified content:
//! permissions, pagination, the not-yet-verified catalog and implicit sets.
use super::*;
use meshway_rules::catalog::{self, Catalog, ConversionRecord, Location, SetRecord};

const SETS: [(&str, meshway_rules::convert::Format, &str); 4] = [
    (
        "openai",
        meshway_rules::convert::Format::Classical,
        "DOMAIN,exact.example\nDOMAIN-SUFFIX,openai.com\nDOMAIN-SUFFIX,chatgpt.com\n\
         DOMAIN-KEYWORD,tracker\nIP-CIDR,10.0.0.0/24\nIP-CIDR,10.0.2.0/24\nIP-CIDR6,2001:db8::/32\n",
    ),
    (
        "openai@ads",
        meshway_rules::convert::Format::MihomoDomainText,
        "+.ads.openai.com\n",
    ),
    (
        "opera",
        meshway_rules::convert::Format::MihomoDomainText,
        "+.opera.com\n",
    ),
    (
        "zoom",
        meshway_rules::convert::Format::MihomoDomainText,
        "+.zoom.us\n",
    ),
];

/// Serves a catalog of `SETS`, with the `openai` source replaced when
/// given, and returns it.
fn publish(
    remote: &HttpsSource,
    key: &ed25519_dalek::SigningKey,
    previous: Option<&Catalog>,
    openai: Option<&str>,
) -> Catalog {
    let mut records = BTreeMap::new();
    let mut artifacts = BTreeMap::new();
    for (name, format, source) in SETS {
        let source = match (name, openai) {
            ("openai", Some(replacement)) => replacement,
            _ => source,
        };
        let conversion = meshway_rules::convert::convert(source.as_bytes(), format).unwrap();
        let record = SetRecord {
            upstream: Location {
                repo: "corp/rules".into(),
                reference: "main".into(),
                commit: "0123456789abcdef0123456789abcdef01234567".into(),
                path: format!("{name}.list"),
            },
            conversion: ConversionRecord::new(source.as_bytes(), format, &conversion),
            held: None,
        };
        let digest = conversion.artifact.digest();
        let bytes = conversion.artifact.bytes().to_vec();
        remote.route(
            &format!("/dist/artifacts/{}", digest.file_name()),
            200,
            &bytes,
        );
        remote.route(
            &format!("/dist/sets/corp/{name}.json"),
            200,
            &serde_json::to_vec(&record).unwrap(),
        );
        artifacts.insert(digest, bytes);
        records.insert(format!("corp/{name}"), record);
    }
    let catalog = Catalog::assemble(previous, records, chrono::Utc::now(), |digest| {
        meshway_rules::RuleSet::from_vec(artifacts[digest].clone())
    })
    .unwrap();
    let index = catalog.to_bytes();
    remote.route("/dist/index.json", 200, &index);
    remote.route(
        "/dist/index.json.sig",
        200,
        &serde_json::to_vec(&catalog::sign(&index, key)).unwrap(),
    );
    catalog
}

async fn status(request: RequestBuilder) -> (StatusCode, String) {
    let response = request.send().await.unwrap();
    (response.status(), response.text().await.unwrap())
}

#[tokio::test]
async fn catalog_sets_members_and_matches_are_read_from_verified_content() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let key = catalog::signing_key(&"33".repeat(32)).unwrap();
    let first = publish(&remote, &key, None, None);
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("browse-owner@example.com").await;
    let member = s.register("browse-member@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let get = |path: &str, bearer: &str| {
        s.client
            .get(s.network(network, path))
            .bearer_auth(bearer.to_owned())
    };

    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.rule_set_providers.insert(
        "corp".into(),
        Some(RuleSetProvider {
            catalog: remote.url("/dist/index.json"),
            collection: "corp".into(),
            public_keys: vec![catalog::public_key(&key).to_string()],
            update_interval: "24h".parse().unwrap(),
        }),
    );
    input.rule_sets.insert(
        "office".into(),
        RuleSet::Inline {
            ip_cidr: vec![],
            domain_suffix: vec![],
            domain: vec![],
            domain_subdomain: vec!["cdn.example".into()],
        },
    );
    let applied = s.apply(token, network, current.revision, &input).await;

    // Before any set of a provider is prepared its catalog is not verified:
    // an explicit problem, never an empty list, and no download.
    let requests = remote.requests.load(Ordering::SeqCst);
    for provider in ["corp", "geosite"] {
        let (code, body) =
            status(get(&format!("rule-set-catalog?provider={provider}"), token)).await;
        assert_eq!(code, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("rule_set_catalog_not_ready"), "{body}");
    }
    let (code, _) = status(get("rule-set-catalog?provider=nope", token)).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert_eq!(remote.requests.load(Ordering::SeqCst), requests);

    // Referencing one set verifies and keeps the catalog.
    let mut referencing = input.clone();
    referencing.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["corp:openai".into(), "office".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    s.apply(token, network, applied.revision, &referencing)
        .await;
    let requests = remote.requests.load(Ordering::SeqCst);

    let page: RuleSetCatalogPage = json(get("rule-set-catalog?provider=corp", token)).await;
    assert_eq!(page.total, 4);
    assert_eq!(page.provider.sequence, 1);
    assert_eq!(page.provider.collection, "corp");
    assert_eq!(
        page.items
            .iter()
            .map(|i| i.reference.as_str())
            .collect::<Vec<_>>(),
        ["corp:openai", "corp:openai@ads", "corp:opera", "corp:zoom"]
    );
    let openai = &page.items[0];
    assert_eq!(
        (
            openai.name.as_str(),
            openai.set.as_str(),
            openai.upstream.path.as_str()
        ),
        ("openai", "corp/openai", "openai.list")
    );
    assert_eq!(openai.digest, first.sets["corp/openai"].digest.to_string());
    assert_eq!(openai.entries, 7);
    assert!(openai.held.is_none() && page.next_cursor.is_none());

    // Case-insensitive name prefix, paged one at a time.
    let first_page: RuleSetCatalogPage = json(get(
        "rule-set-catalog?provider=corp&q=OPENAI&limit=1",
        token,
    ))
    .await;
    assert_eq!(first_page.total, 2);
    assert_eq!(first_page.items[0].name, "openai");
    let cursor = first_page.next_cursor.clone().unwrap();
    let second: RuleSetCatalogPage = json(get(
        &format!("rule-set-catalog?provider=corp&q=OPENAI&limit=1&cursor={cursor}"),
        token,
    ))
    .await;
    assert_eq!(second.items[0].name, "openai@ads");
    assert!(second.next_cursor.is_none());
    let (code, body) = status(get(
        &format!("rule-set-catalog?provider=corp&q=op&limit=1&cursor={cursor}"),
        token,
    ))
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(body.contains("invalid_cursor"), "{body}");
    let (code, body) = status(get("rule-set-catalog?provider=corp&cursor=AAAA", token)).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(body.contains("invalid_cursor"), "{body}");
    let (code, _) = status(get("rule-set-catalog?provider=corp&limit=0", token)).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let none: RuleSetCatalogPage = json(get("rule-set-catalog?provider=corp&q=x", token)).await;
    assert!(none.items.is_empty() && none.total == 0);

    // Members of the implicit set, one kind per request.
    let entries = |query: &str| get(&format!("rule-sets/corp:openai/entries?{query}"), token);
    let domains: RuleSetEntryPage = json(entries("kind=domain&limit=2")).await;
    assert_eq!(domains.digest, openai.digest);
    assert_eq!(
        (
            domains.counts.exact,
            domains.counts.suffix,
            domains.counts.keyword
        ),
        (1, 2, 1)
    );
    assert_eq!((domains.counts.ipv4, domains.counts.ipv6), (2, 1));
    // Reversed-name order: `elpmaxe.tcaxe` < `moc.ianepo` < `moc.tpgtahc`.
    assert_eq!(
        domains.items,
        [
            RuleSetMember::Exact {
                name: "exact.example".into()
            },
            RuleSetMember::Suffix {
                name: "openai.com".into()
            },
        ]
    );
    let rest: RuleSetEntryPage = json(entries(&format!(
        "kind=domain&limit=2&cursor={}",
        domains.next_cursor.unwrap()
    )))
    .await;
    assert_eq!(
        rest.items,
        [RuleSetMember::Suffix {
            name: "chatgpt.com".into()
        }]
    );
    assert!(rest.next_cursor.is_none());
    let suffixed: RuleSetEntryPage = json(entries("kind=domain&q=OpenAI.com.")).await;
    assert_eq!(suffixed.items.len(), 1);
    let below: RuleSetEntryPage = json(entries("kind=domain&q=.openai.com")).await;
    assert!(below.items.is_empty());
    let keywords: RuleSetEntryPage = json(entries("kind=keyword&q=TRA")).await;
    assert_eq!(
        keywords.items,
        [RuleSetMember::Keyword {
            keyword: "tracker".into()
        }]
    );
    let v4: RuleSetEntryPage = json(entries("kind=ipv4&q=10.0.1.7")).await;
    assert_eq!(
        v4.items,
        [RuleSetMember::IpRange {
            first: "10.0.2.0".into(),
            last: "10.0.2.255".into()
        }]
    );
    let v4_all: RuleSetEntryPage = json(entries("kind=ipv4&limit=1")).await;
    let v4_next: RuleSetEntryPage = json(entries(&format!(
        "kind=ipv4&limit=1&cursor={}",
        v4_all.next_cursor.unwrap()
    )))
    .await;
    assert_eq!(v4_next.items, v4.items);
    let v6: RuleSetEntryPage = json(entries("kind=ipv6")).await;
    assert_eq!(
        v6.items,
        [RuleSetMember::IpRange {
            first: "2001:db8::".into(),
            last: "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff".into()
        }]
    );
    for bad in ["kind=ipv4&q=2001:db8::1", "kind=other", "q=x"] {
        let (code, body) = status(entries(bad)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }

    // Match queries name the deciding member.
    let matched = |path: &str, query: &str| {
        let get = &get;
        let url = format!("rule-sets/{path}/match?{query}");
        async move { json::<RuleSetMatchResponse>(get(&url, token)).await }
    };
    let hit = matched("corp:openai", "name=API.OpenAI.com.").await;
    assert_eq!(hit.query, "api.openai.com");
    assert_eq!(
        hit.member,
        Some(RuleSetMember::Suffix {
            name: "openai.com".into()
        })
    );
    assert_eq!(
        matched("corp:openai", "name=_dmarc.openai.com")
            .await
            .member,
        Some(RuleSetMember::Suffix {
            name: "openai.com".into()
        })
    );
    assert_eq!(
        matched("corp:openai", "name=exact.example").await.member,
        Some(RuleSetMember::Exact {
            name: "exact.example".into()
        })
    );
    assert_eq!(
        matched("corp:openai", "name=sub.exact.example")
            .await
            .member,
        None
    );
    assert_eq!(
        matched("corp:openai", "name=mytracker.net").await.member,
        Some(RuleSetMember::Keyword {
            keyword: "tracker".into()
        })
    );
    assert_eq!(
        matched("corp:openai", "ip=10.0.2.9").await.member,
        Some(RuleSetMember::IpRange {
            first: "10.0.2.0".into(),
            last: "10.0.2.255".into()
        })
    );
    assert_eq!(matched("corp:openai", "ip=10.0.1.1").await.member, None);
    assert_eq!(
        matched("office", "name=x.cdn.example").await.member,
        Some(RuleSetMember::Subdomain {
            name: "cdn.example".into()
        })
    );
    assert_eq!(matched("office", "name=cdn.example").await.member, None);
    let subdomains: RuleSetEntryPage =
        json(get("rule-sets/office/entries?kind=domain", token)).await;
    assert_eq!(
        subdomains.items,
        [RuleSetMember::Subdomain {
            name: "cdn.example".into()
        }]
    );
    for bad in ["", "name=a.example&ip=10.0.0.1", "ip=10.0.0", "name=."] {
        let (code, body) = status(get(&format!("rule-sets/corp:openai/match?{bad}"), token)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    for path in [
        "rule-sets/missing/entries?kind=domain",
        "rule-sets/missing/match?name=a.example",
    ] {
        let (code, _) = status(get(path, token)).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "{path}");
    }
    // Reading never downloads.
    assert_eq!(remote.requests.load(Ordering::SeqCst), requests);

    // A new catalog sequence or binding ends the old cursors.
    let catalog_cursor = first_page.next_cursor.unwrap();
    let entry_cursor = json::<RuleSetEntryPage>(entries("kind=ipv4&limit=1"))
        .await
        .next_cursor
        .unwrap();
    publish(
        &remote,
        &key,
        Some(&first),
        Some("DOMAIN-SUFFIX,openai.com\n"),
    );
    let refreshed: RuleSetView = json(
        s.client
            .post(s.network(network, "rule-sets/corp:openai/refresh"))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id: rid() }),
    )
    .await;
    assert_eq!(refreshed.problem, None);
    for path in [
        format!("rule-set-catalog?provider=corp&q=OPENAI&limit=1&cursor={catalog_cursor}"),
        format!("rule-sets/corp:openai/entries?kind=ipv4&limit=1&cursor={entry_cursor}"),
    ] {
        let (code, body) = status(get(&path, token)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(body.contains("invalid_cursor"), "{body}");
    }
    let newer: RuleSetCatalogPage = json(get("rule-set-catalog?provider=corp", token)).await;
    assert_eq!(newer.provider.sequence, 2);

    // Members and Nodes cannot read resource content.
    let link: JoinLinkCreated = json(
        s.client
            .post(s.network(network, "join-links"))
            .bearer_auth(token)
            .json(&CreateJoinLinkRequest {
                request_id: rid(),
                role: MembershipRole::Member,
                expires_in_seconds: 600,
            }),
    )
    .await;
    let JoinLinkCreated::Available { secret, .. } = link else {
        panic!("new link");
    };
    let _: JoinLinkRedeemed = json(
        s.client
            .post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(member.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            }),
    )
    .await;
    let (_, node_token, _) = enroll(&s, token, network, 72).await;
    for path in [
        "rule-set-catalog?provider=corp",
        "rule-sets/corp:openai/entries?kind=domain",
        "rule-sets/corp:openai/match?name=openai.com",
    ] {
        let (code, _) = status(get(path, member.token.expose())).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{path}");
        let (code, _) = status(get(path, &node_token)).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "{path}");
    }
    s.close().await;
}
