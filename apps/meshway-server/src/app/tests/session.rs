//! Fresh-proof session recovery over genuine enrolled identities and owner jobs.
use super::{
    node::{enroll_user_node, fixture},
    *,
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
    execution::{Execution, proof},
    store,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::{
    ChallengeId, Digest, InstanceId, KeyId, MembershipRole, NetworkId, NodeId, RequestId,
};
use meshway_protocol::{WireSecret, node::*};
use rusqlite::params;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn intent() -> NodeSessionIntent {
    NodeSessionIntent { request_id: rid() }
}
async fn challenge(
    s: &Execution,
    network: NetworkId,
    node: NodeId,
    keys: &NodeKeyMetadata,
    intent: &NodeSessionIntent,
) -> NodeChallenge {
    s.challenge(NodeChallengeRequest {
        network_id: network,
        node_id: node,
        key_id: keys.control.id,
        operation: ProofOperation::Session,
        request_digest: proof::request_digest(intent).unwrap(),
    })
    .await
    .unwrap()
}
fn signed(
    c: &NodeChallenge,
    intent: &NodeSessionIntent,
    signer: &SigningKey,
) -> NodeSessionRequest {
    NodeSessionRequest {
        intent: intent.clone(),
        proof: NodeProof {
            challenge_id: c.id,
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(signer.sign(&proof_transcript(c)).to_bytes()),
            ),
        },
    }
}
fn hash(session: &NodeSessionResponse) -> [u8; 32] {
    proof::token_hash(session.token.expose()).unwrap()
}
async fn counts(app: &App, node: NodeId) -> (i64, i64, i64) {
    app.executor.handle().execute(move |db| {
        let key = node.as_uuid().as_bytes().as_slice();
        let sessions = db.query_row("SELECT count(*) FROM node_sessions WHERE node_id=?1", [key], |r| r.get(0)).unwrap();
        let anchors = db.query_row("SELECT count(*) FROM node_challenges WHERE node_id=?1 AND operation='session' AND consumed_at IS NOT NULL", [key], |r| r.get(0)).unwrap();
        let proofs = db.query_row("SELECT count(*) FROM node_challenges WHERE node_id=?1 AND consumed_at IS NULL", [key], |r| r.get(0)).unwrap();
        (sessions, anchors, proofs)
    }).await.unwrap()
}

#[tokio::test]
async fn fresh_proof_recovers_original_session_at_capacity_but_consumed_proof_never_replays() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let s = app.execution();
    let first_intent = intent();
    let first = challenge(&s, network, a, &keys, &first_intent).await;
    let issued = s
        .session(signed(&first, &first_intent, &signer))
        .await
        .unwrap();
    assert_eq!(counts(&app, a).await, (1, 1, 0));
    assert_eq!(
        s.session(signed(&first, &first_intent, &signer))
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    for _ in 0..12 {
        let c = challenge(&s, network, a, &keys, &first_intent).await;
        let recovered = s.session(signed(&c, &first_intent, &signer)).await.unwrap();
        assert_eq!(hash(&recovered), hash(&issued));
        assert_eq!(recovered.expires_at_ms, issued.expires_at_ms);
        assert_eq!(
            s.session(signed(&c, &first_intent, &signer)).await.err(),
            Some(Error::InvalidCredentials)
        );
        assert_eq!(counts(&app, a).await, (1, 1, 0));
    }
    // Anchors cannot consume the proof slots needed for authenticated recovery.
    for _ in 1..8 {
        let next = intent();
        let c = challenge(&s, network, a, &keys, &next).await;
        let other = s.session(signed(&c, &next, &signer)).await.unwrap();
        assert_ne!(hash(&other), hash(&issued));
    }
    assert_eq!(counts(&app, a).await, (8, 8, 0));
    let overflow = intent();
    let c = challenge(&s, network, a, &keys, &overflow).await;
    assert_eq!(
        s.session(signed(&c, &overflow, &signer)).await.err(),
        Some(Error::Busy)
    );
    let retry = challenge(&s, network, a, &keys, &first_intent).await;
    let recovered = s
        .session(signed(&retry, &first_intent, &signer))
        .await
        .unwrap();
    assert_eq!(hash(&recovered), hash(&issued));
    assert_eq!(recovered.expires_at_ms, issued.expires_at_ms);
    assert_eq!(counts(&app, a).await, (8, 8, 1));

    // Removing one live row at capacity must prune just its orphaned anchor,
    // then mint from the new challenge rather than reviving its captured token.
    let old_hash = hash(&issued);
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "DELETE FROM node_sessions WHERE token_hash=?1",
                [old_hash.as_slice()],
            )
            .unwrap();
        })
        .await
        .unwrap();
    let fresh = challenge(&s, network, a, &keys, &first_intent).await;
    let replacement = s
        .session(signed(&fresh, &first_intent, &signer))
        .await
        .unwrap();
    assert_ne!(hash(&replacement), old_hash);
    assert!(s.authenticate(issued.token.expose()).await.is_err());
    s.authenticate(replacement.token.expose()).await.unwrap();
    assert_eq!(counts(&app, a).await, (8, 8, 1));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_proof_anchor_survives_cleanup_but_only_a_fresh_proof_can_recover() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let s = app.execution();
    let intent = intent();
    let mut anchor = challenge(&s, network, a, &keys, &intent).await;
    // Shorten only this genuine challenge's lifetime before it is signed. Its
    // nonce, identity and signature verification still come from normal flows.
    anchor.expires_at_ms = u64::try_from(store::execution::now().unwrap()).unwrap() + 2000;
    let expires = anchor.expires_at_ms;
    let id = anchor.id;
    app.executor
        .handle()
        .execute(move |db| {
            db.execute(
                "UPDATE node_challenges SET expires_at=?1 WHERE id=?2",
                params![
                    i64::try_from(expires).unwrap(),
                    id.as_uuid().as_bytes().as_slice()
                ],
            )
            .unwrap();
        })
        .await
        .unwrap();
    let issued = s.session(signed(&anchor, &intent, &signer)).await.unwrap();
    let now = u64::try_from(store::execution::now().unwrap()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(
        expires.saturating_sub(now) + 1,
    ))
    .await;
    assert_eq!(
        s.session(signed(&anchor, &intent, &signer)).await.err(),
        Some(Error::InvalidCredentials)
    );
    let fresh = challenge(&s, network, a, &keys, &intent).await;
    assert_eq!(counts(&app, a).await, (1, 1, 1));
    let recovered = s.session(signed(&fresh, &intent, &signer)).await.unwrap();
    assert_eq!(hash(&recovered), hash(&issued));
    assert_eq!(recovered.expires_at_ms, issued.expires_at_ms);
    assert_eq!(counts(&app, a).await, (1, 1, 0));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_intent_and_new_server_boot_issue_new_tokens_without_old_bearer_resurrection() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let s = app.execution();
    let intent = intent();
    let c = challenge(&s, network, a, &keys, &intent).await;
    let first = s.session(signed(&c, &intent, &signer)).await.unwrap();
    let first_hash = hash(&first);
    let anchor = c.id;
    app.executor
        .handle()
        .execute(move |db| {
            let now = store::execution::now().unwrap();
            let tx = db.transaction().unwrap();
            // Advance this already-issued session/anchor to the expired state.
            // Identity, intent, nonce and the signed proof transcript are untouched.
            tx.execute(
                "UPDATE node_sessions SET created_at=?1,expires_at=?2 WHERE token_hash=?3",
                params![now - 300_001, now - 1, first_hash.as_slice()],
            )
            .unwrap();
            tx.execute(
                "UPDATE node_challenges SET consumed_at=?1 WHERE id=?2",
                params![now - 300_001, anchor.as_uuid().as_bytes().as_slice()],
            )
            .unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    let fresh = challenge(&s, network, a, &keys, &intent).await;
    let second = s.session(signed(&fresh, &intent, &signer)).await.unwrap();
    assert_ne!(hash(&second), first_hash);
    assert!(s.authenticate(first.token.expose()).await.is_err());
    assert_eq!(counts(&app, a).await, (1, 1, 0));
    drop(s);
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    let s = app.execution();
    let fresh = challenge(&s, network, a, &keys, &intent).await;
    let third = s.session(signed(&fresh, &intent, &signer)).await.unwrap();
    assert_ne!(hash(&third), hash(&second));
    assert!(s.authenticate(first.token.expose()).await.is_err());
    assert!(s.authenticate(second.token.expose()).await.is_err());
    s.authenticate(third.token.expose()).await.unwrap();
    assert_eq!(counts(&app, a).await, (1, 1, 0));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn recovery_is_scoped_to_current_key_node_intent_and_revocation() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let s = app.execution();
    let intent = intent();
    let ca = challenge(&s, network, a, &ak, &intent).await;
    let issued_a = s.session(signed(&ca, &intent, &signer_a)).await.unwrap();
    let cb = challenge(&s, network, b, &bk, &intent).await;
    assert_eq!(
        s.session(signed(&cb, &intent, &signer_a)).await.err(),
        Some(Error::InvalidCredentials)
    );
    let issued_b = s.session(signed(&cb, &intent, &signer_b)).await.unwrap();
    assert_ne!(hash(&issued_a), hash(&issued_b));
    assert_eq!(issued_b.node_id, b);
    let pending = challenge(&s, network, a, &ak, &intent).await;
    let wrong_intent = NodeSessionIntent {
        request_id: RequestId::new(random_uuid().unwrap()).unwrap(),
    };
    assert_eq!(
        s.session(signed(&pending, &wrong_intent, &signer_a))
            .await
            .err(),
        Some(Error::InvalidCredentials)
    );
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    s.revoke_key(
        owner.token.expose(),
        network,
        a,
        RevokeNodeKeyRequest {
            key_id: ak.control.id,
            expected_generation: ak.control.generation,
        },
    )
    .await
    .unwrap();
    assert!(
        s.session(signed(&pending, &intent, &signer_a))
            .await
            .is_err()
    );
    assert!(s.authenticate(issued_a.token.expose()).await.is_err());
    s.authenticate(issued_b.token.expose()).await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn membership_removal_rejects_fresh_signed_recovery_of_a_previously_live_session() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, _, _, _, _, _, _) = fixture().await;
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            intent().request_id,
        )
        .await
        .unwrap();
    let user = app
        .accounts()
        .register(
            "session-member@example.test".into(),
            Secret::new("a sufficiently long password".into()),
            intent().request_id,
        )
        .await
        .unwrap();
    let link = app
        .networks()
        .create_join_link(
            owner.token.expose(),
            network,
            MembershipRole::Member,
            600,
            intent().request_id,
        )
        .await
        .unwrap();
    let (_, membership) = app
        .networks()
        .redeem_join_link(
            user.token.expose(),
            link.secret.unwrap().expose(),
            intent().request_id,
        )
        .await
        .unwrap();
    let signer = SigningKey::from_bytes(&[83; 32]);
    let node = enroll_user_node(&app, user.token.expose(), network, &signer, [84; 32]).await;
    let s = app.execution();
    let intent = intent();
    let c = challenge(&s, network, node.node_id, &node.keys, &intent).await;
    let session = s.session(signed(&c, &intent, &signer)).await.unwrap();
    let pending = challenge(&s, network, node.node_id, &node.keys, &intent).await;
    let impact = app
        .networks()
        .removal_impact(owner.token.expose(), network, Some(membership), rid())
        .await
        .unwrap();
    app.networks()
        .remove_member(
            owner.token.expose(),
            network,
            Some(membership),
            impact.version,
            impact.revision,
            rid(),
        )
        .await
        .unwrap();
    assert!(s.session(signed(&pending, &intent, &signer)).await.is_err());
    assert!(s.authenticate(session.token.expose()).await.is_err());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn token_derivation_binds_boot_installation_and_full_unique_challenge_transcript() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, _, keys, _, signer, _) = fixture().await;
    let s = app.execution();
    let intent = intent();
    let anchor = challenge(&s, network, a, &keys, &intent).await;
    let issued = s.session(signed(&anchor, &intent, &signer)).await.unwrap();
    let sealer = app.sealer();
    let (token, hash) = proof::session_token(&sealer, app.boot, &anchor).unwrap();
    assert_eq!(hash, proof::token_hash(token.expose()).unwrap());
    assert_eq!(hash, self::hash(&issued));
    let boot = InstanceId::new(random_uuid().unwrap()).unwrap();
    assert_ne!(
        proof::session_token(&sealer, boot, &anchor).unwrap().1,
        hash
    );
    let mut variants = vec![anchor.clone(); 6];
    variants[0].network_id = NetworkId::new(random_uuid().unwrap()).unwrap();
    variants[1].id = ChallengeId::new(random_uuid().unwrap()).unwrap();
    variants[2].request_digest = Digest::from_bytes([91; 32]);
    variants[3].key_generation = anchor.key_generation.checked_next().unwrap();
    variants[4].operation = ProofOperation::Rotate;
    variants[5].controller_id = KeyId::new(random_uuid().unwrap()).unwrap();
    for variant in variants {
        assert_ne!(
            proof::session_token(&sealer, app.boot, &variant).unwrap().1,
            hash
        );
    }
    let mut context = app.boot.as_uuid().as_bytes().to_vec();
    context.extend_from_slice(&proof_transcript(&anchor));
    let other_domain = sealer.private_digest(b"meshway/credential-fingerprint/v1\0", &context);
    let other_token = format!("mwns_{}", URL_SAFE_NO_PAD.encode(other_domain));
    assert_ne!(proof::token_hash(&other_token).unwrap(), hash);
    app.shutdown().await.unwrap();
}
