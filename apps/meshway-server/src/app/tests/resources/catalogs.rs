//! Provider catalogs the Server keeps current by itself: fetched before any
//! set is referenced, once per (URL, trusted keys) across networks, with the
//! last accepted catalog kept through failures and regressions.
use super::*;
use meshway_rules::catalog;

const INDEX: &str = "/dist/index.json";

async fn catalog_page(
    s: &Server,
    token: &str,
    network: NetworkId,
) -> (StatusCode, Option<RuleSetCatalogPage>) {
    let response = s
        .client
        .get(s.network(network, "rule-set-catalog?provider=corp"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let code = response.status();
    let body = response.text().await.unwrap();
    if code == StatusCode::CONFLICT {
        assert!(body.contains("rule_set_catalog_not_ready"), "{body}");
    }
    (code, serde_json::from_str(&body).ok())
}

/// Waits until the Server fetched `index.json` more than `count` times.
async fn fetched_beyond(remote: &HttpsSource, count: usize) -> usize {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let now = remote.fetched(INDEX);
            if now > count {
                return now;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the scheduler fetched the catalog again")
}

async fn sequence_becomes(s: &Server, token: &str, network: NetworkId, sequence: u32) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let (StatusCode::OK, Some(page)) = catalog_page(s, token, network).await
                && page.provider.sequence == sequence
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the catalog reached the sequence")
}

#[tokio::test]
async fn provider_catalogs_are_fetched_unreferenced_once_per_url_and_kept_through_failures() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let key = catalog::signing_key(&"55".repeat(32)).unwrap();
    // The catalog is not published yet.
    remote.route(INDEX, 500, b"unavailable");
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("catalogs-owner@example.com").await;
    let token = owner.token.expose();
    let provider = RuleSetProvider {
        catalog: remote.url(INDEX),
        collection: "corp".into(),
        public_keys: vec![catalog::public_key(&key).to_string()],
        update_interval: "24h".parse().unwrap(),
    };
    // Two networks name the same catalog and trust; neither references a set.
    let mut networks = Vec::new();
    for _ in 0..2 {
        let network = s.create(token).await.id;
        let current = s.source(token, network).await;
        let mut input = current.configuration.clone();
        input
            .rule_set_providers
            .insert("corp".into(), Some(provider.clone()));
        s.apply(token, network, current.revision, &input).await;
        networks.push(network);
    }

    // A fetch that never succeeded is not ready, not an empty list.
    let failed = fetched_beyond(&remote, 0).await;
    for network in &networks {
        assert_eq!(
            catalog_page(&s, token, *network).await.0,
            StatusCode::CONFLICT
        );
    }

    // Published: the backed-off retry accepts it for both networks at once.
    let first = super::browse::publish(&remote, &key, None, None, None);
    sequence_becomes(&s, token, networks[0], 1).await;
    let (code, page) = catalog_page(&s, token, networks[1]).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(page.unwrap().total, 4);
    // One failed and one successful fetch served both networks; the
    // signature travels with the catalog, and no artifact is fetched.
    assert_eq!((failed, remote.fetched(INDEX)), (1, 2));
    assert_eq!(remote.fetched("/dist/index.json.sig"), 1);
    assert!(
        remote
            .paths
            .lock()
            .unwrap()
            .iter()
            .all(|p| !p.starts_with("/dist/artifacts/"))
    );

    // The shortest interval of the networks naming the catalog applies.
    let current = s.source(token, networks[0]).await;
    let mut input = current.configuration.clone();
    input.rule_set_providers.insert(
        "corp".into(),
        Some(RuleSetProvider {
            update_interval: "1s".parse().unwrap(),
            ..provider.clone()
        }),
    );
    s.apply(token, networks[0], current.revision, &input).await;
    fetched_beyond(&remote, 2).await;

    // A failed fetch keeps the accepted catalog.
    remote.route(INDEX, 500, b"unavailable");
    let before = remote.fetched(INDEX);
    fetched_beyond(&remote, before).await;
    let (code, page) = catalog_page(&s, token, networks[0]).await;
    assert_eq!((code, page.unwrap().provider.sequence), (StatusCode::OK, 1));

    // A newer catalog replaces it after the backoff.
    let second = super::browse::publish(&remote, &key, Some(&first), None, None);
    assert_eq!(second.sequence, 2);
    sequence_becomes(&s, token, networks[1], 2).await;

    // A lower sequence, even correctly signed, never replaces it.
    let index = first.to_bytes();
    remote.route(INDEX, 200, &index);
    remote.route(
        "/dist/index.json.sig",
        200,
        &serde_json::to_vec(&catalog::sign(&index, &key)).unwrap(),
    );
    let before = remote.fetched(INDEX);
    fetched_beyond(&remote, before).await;
    let (code, page) = catalog_page(&s, token, networks[0]).await;
    assert_eq!((code, page.unwrap().provider.sequence), (StatusCode::OK, 2));

    // A held set: the catalog page and the referencing set's view both say
    // so, with the pipeline's reason and first hold time.
    let held = catalog::Held {
        since: "2026-10-12T03:17:00Z".parse().unwrap(),
        reason: "entries dropped from 4 to 1".into(),
    };
    let third = super::browse::publish(&remote, &key, Some(&second), None, Some(held.clone()));
    sequence_becomes(&s, token, networks[0], third.sequence as u32).await;
    let (_, page) = catalog_page(&s, token, networks[0]).await;
    let page = page.unwrap();
    let openai = page
        .items
        .iter()
        .find(|item| item.name == "openai")
        .unwrap();
    let expected = RuleSetHeld {
        since: held.since,
        reason: held.reason.clone(),
    };
    assert_eq!(openai.held.as_ref(), Some(&expected));
    assert_eq!(openai.upstream.reference, openai.upstream.commit);
    assert!(
        page.items
            .iter()
            .filter(|item| item.name != "openai")
            .all(|item| item.held.is_none())
    );
    let current = s.source(token, networks[1]).await;
    let mut input = current.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["corp:openai".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    s.apply(token, networks[1], current.revision, &input).await;
    let view = json::<RuleSetList>(
        s.client
            .get(s.network(networks[1], "rule-sets/corp:openai"))
            .bearer_auth(token),
    )
    .await
    .items
    .remove(0);
    assert_eq!(view.held, Some(expected));
    s.close().await;
}
