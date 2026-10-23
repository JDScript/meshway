//! Catalog providers end to end: `<provider>:<name>` expansion, signature and
//! sequence checks, the resource view and the Node artifact endpoint.
use super::*;
use meshway_rules::catalog::{self, Catalog, ConversionRecord, Location, SetRecord};

struct Published {
    index: Vec<u8>,
    digest: meshway_rules::Digest,
    artifact: Vec<u8>,
    record: Vec<u8>,
}

fn publish(source: &str, previous: Option<&Catalog>) -> (Catalog, Published) {
    publish_at(source, previous, "0123456789abcdef0123456789abcdef01234567")
}

fn publish_at(source: &str, previous: Option<&Catalog>, commit: &str) -> (Catalog, Published) {
    let conversion = meshway_rules::convert::convert(
        source.as_bytes(),
        meshway_rules::convert::Format::MihomoDomainText,
    )
    .unwrap();
    let artifact = conversion.artifact.bytes().to_vec();
    let record = SetRecord {
        upstream: Location {
            repo: "corp/rules".into(),
            reference: "main".into(),
            commit: commit.into(),
            path: "openai.list".into(),
        },
        conversion: ConversionRecord::new(
            source.as_bytes(),
            meshway_rules::convert::Format::MihomoDomainText,
            &conversion,
        ),
        held: None,
    };
    let records: BTreeMap<_, _> = [("corp/openai".to_owned(), record.clone())].into();
    let catalog = Catalog::assemble(previous, records, chrono::Utc::now(), |_| {
        meshway_rules::RuleSet::from_vec(artifact.clone())
    })
    .unwrap();
    let published = Published {
        index: catalog.to_bytes(),
        digest: conversion.artifact.digest(),
        artifact,
        record: serde_json::to_vec(&record).unwrap(),
    };
    (catalog, published)
}

fn serve(remote: &HttpsSource, published: &Published, key: &ed25519_dalek::SigningKey) {
    let signature = serde_json::to_vec(&catalog::sign(&published.index, key)).unwrap();
    remote.route("/dist/index.json", 200, &published.index);
    remote.route("/dist/index.json.sig", 200, &signature);
    remote.route(
        &format!("/dist/artifacts/{}", published.digest.file_name()),
        200,
        &published.artifact,
    );
    remote.route("/dist/sets/corp/openai.json", 200, &published.record);
}

fn provider(remote: &HttpsSource, key: &ed25519_dalek::SigningKey) -> RuleSetProvider {
    RuleSetProvider {
        catalog: remote.url("/dist/index.json"),
        collection: "corp".into(),
        public_keys: vec![catalog::public_key(key).to_string()],
        update_interval: "24h".parse().unwrap(),
    }
}

async fn implicit(s: &Server, token: &str, network: NetworkId) -> RuleSetView {
    json::<RuleSetList>(
        s.client
            .get(s.network(network, "rule-sets/corp:openai"))
            .bearer_auth(token),
    )
    .await
    .items
    .remove(0)
}

async fn refresh(s: &Server, token: &str, network: NetworkId) -> RuleSetView {
    json(
        s.client
            .post(s.network(network, "rule-sets/corp:openai/refresh"))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id: rid() }),
    )
    .await
}

#[tokio::test]
async fn provider_sets_are_verified_bound_by_digest_and_served_only_to_referencing_nodes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let key = catalog::signing_key(&"11".repeat(32)).unwrap();
    let other = catalog::signing_key(&"22".repeat(32)).unwrap();
    let (first, published) = publish("+.openai.com\nchatgpt.com\n", None);
    let (second, newer) = publish("+.openai.com\nchatgpt.com\n", Some(&first));
    serve(&remote, &newer, &key);
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("providers@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (node, node_token, _) = enroll(&s, token, network, 71).await;

    // An untrusted key rejects the edit; nothing is bound.
    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
    input
        .rule_set_providers
        .insert("corp".into(), Some(provider(&remote, &other)));
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["corp:openai".into()],
            ..Default::default()
        },
        action: RuleAction::Direct,
    });
    let rejected = s
        .client
        .post(s.network(network, "configuration"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected.text().await.unwrap().contains("signature_invalid"));
    // A name the catalog does not have is an unknown rule set at the reference.
    let mut unknown = input.clone();
    unknown
        .rule_set_providers
        .insert("corp".into(), Some(provider(&remote, &key)));
    unknown.routing.egress.rules[0].matcher.rule_sets = vec!["corp:missing".into()];
    let rejected = s
        .client
        .post(s.network(network, "configuration"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&unknown).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = rejected.text().await.unwrap();
    assert!(
        text.contains("unknown_rule_set")
            && text.contains("/routing/egress/rules/0/match/ruleSets/0"),
        "{text}"
    );

    // The trusted key binds the set; the configuration text keeps only the reference.
    input
        .rule_set_providers
        .insert("corp".into(), Some(provider(&remote, &key)));
    s.apply(token, network, current.revision, &input).await;
    assert!(
        !s.source(token, network)
            .await
            .configuration
            .rule_sets
            .contains_key("corp:openai")
    );
    let view = implicit(&s, token, network).await;
    assert!(view.implicit && view.definition.is_none());
    assert_eq!(view.problem, None);
    let provider_view = view.provider.clone().unwrap();
    assert_eq!(
        (
            provider_view.name.as_str(),
            provider_view.collection.as_str(),
            provider_view.sequence
        ),
        ("corp", "corp", Some(2))
    );
    assert_eq!(
        view.source.as_ref().unwrap().repo.as_deref(),
        Some("corp/rules")
    );
    assert_eq!(
        view.artifact.as_ref().unwrap().digest,
        newer.digest.to_string()
    );
    assert_eq!(view.conversion.as_ref().unwrap().converted.suffix, 1);

    // The Node's netmap references the digest; the Node may fetch exactly it.
    let artifact = |digest: String, token: String| {
        s.client
            .get(format!("{}/agent-api/v1/artifacts/{digest}", s.url))
            .bearer_auth(token)
            .send()
    };
    let response = artifact(newer.digest.to_string(), node_token.clone())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        meshway_rules::MEDIA_TYPE
    );
    assert_eq!(
        response.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        newer.artifact.as_slice()
    );
    // In-flight downloads of one Node are bounded: two admitted and still
    // held (their bodies unsent), a third is busy with Retry-After until
    // one is finished or dropped.
    let netmaps = s.app.netmaps();
    let first_held = netmaps
        .artifact(&node_token, newer.digest.to_string())
        .await
        .unwrap();
    let second_held = netmaps
        .artifact(&node_token, newer.digest.to_string())
        .await
        .unwrap();
    assert_eq!(first_held.bytes, newer.artifact);
    let busy = artifact(newer.digest.to_string(), node_token.clone())
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(busy.headers()["retry-after"], "1");
    assert_eq!(
        busy.json::<serde_json::Value>().await.unwrap()["problem"]["code"],
        "busy"
    );
    drop(first_held);
    let admitted = artifact(newer.digest.to_string(), node_token.clone())
        .await
        .unwrap();
    assert_eq!(admitted.status(), StatusCode::OK);
    assert_eq!(
        admitted.headers()["content-length"],
        newer.artifact.len().to_string()
    );
    assert_eq!(
        admitted.bytes().await.unwrap().as_ref(),
        newer.artifact.as_slice()
    );
    drop(second_held);
    let unreferenced = meshway_rules::Digest::of(b"other");
    for (digest, bearer) in [
        (unreferenced.to_string(), node_token.clone()),
        ("not-a-digest".to_owned(), node_token.clone()),
    ] {
        let response = artifact(digest, bearer).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    // A user session is not a Node credential.
    let response = artifact(newer.digest.to_string(), token.to_owned())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A bad signature keeps the bound content and reports the problem.
    let forged = serde_json::to_vec(&catalog::sign(&newer.index, &other)).unwrap();
    remote.route("/dist/index.json.sig", 200, &forged);
    let failed = refresh(&s, token, network).await;
    assert_eq!(failed.problem, Some(ResourceProblem::SignatureInvalid));
    assert_eq!(failed.artifact.unwrap().digest, newer.digest.to_string());

    // An older catalog (sequence 1) signed by the trusted key is a regression,
    // even with different content.
    let (_, rolled_back) = publish("+.example.org\n", None);
    assert!(first.sequence < second.sequence);
    serve(&remote, &rolled_back, &key);
    let regressed = refresh(&s, token, network).await;
    assert_eq!(regressed.problem, Some(ResourceProblem::CatalogRegressed));
    assert_eq!(regressed.artifact.unwrap().digest, newer.digest.to_string());

    // A newer sequence with new content rebinds; the old digest stays
    // fetchable only while a lease of the superseded netmap can be live.
    let (third_catalog, third) = publish("+.openai.com\n", Some(&second));
    serve(&remote, &third, &key);
    let rebound = refresh(&s, token, network).await;
    assert_eq!(rebound.problem, None);
    assert_eq!(rebound.provider.unwrap().sequence, Some(3));
    let (network_bytes, node_bytes) = (
        network.as_uuid().as_bytes().to_vec(),
        node.node_id.as_uuid().as_bytes().to_vec(),
    );
    let (old, new) = (published.digest, third.digest);
    let (during, after, current) = s
        .app
        .executor
        .handle()
        .read(move |tx| {
            let now = crate::store::execution::now()?;
            let validity = 60_000;
            let referenced = |digest: &meshway_rules::Digest, at: i64| {
                crate::store::resources::artifacts::referenced(
                    tx,
                    &network_bytes,
                    &node_bytes,
                    digest,
                    at,
                    validity,
                )
            };
            Ok((
                referenced(&old, now)?,
                referenced(&old, now + validity + 1)?,
                referenced(&new, now + validity + 1)?,
            ))
        })
        .await
        .unwrap();
    assert!(during && !after && current);

    // A newer sequence with the same content (one digest) from another
    // upstream commit: the binding stays, its current record follows.
    let (_, fourth) = publish_at(
        "+.openai.com\n",
        Some(&third_catalog),
        "fedcba9876543210fedcba9876543210fedcba98",
    );
    assert_eq!(fourth.digest, third.digest);
    serve(&remote, &fourth, &key);
    let refreshed = refresh(&s, token, network).await;
    assert_eq!(refreshed.problem, None);
    assert_eq!(refreshed.provider.unwrap().sequence, Some(4));
    assert_eq!(
        refreshed.source.unwrap().commit.as_deref(),
        Some("fedcba9876543210fedcba9876543210fedcba98")
    );
    let view = implicit(&s, token, network).await;
    assert_eq!(view.provider.unwrap().sequence, Some(4));
    assert_eq!(view.artifact.unwrap().digest, third.digest.to_string());
    s.close().await;
}
