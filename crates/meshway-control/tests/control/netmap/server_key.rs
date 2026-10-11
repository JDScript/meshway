//! Server signing-key rotation: the pinned key moves only to its endorsed
//! next generation; afterwards only the successor's signatures verify, and a
//! Node that missed the window keeps its held lease but rejects new answers.
use super::{
    HOUR, enrolled,
    support::{Facts, compiled, full, server_key, sign_with, signed, unchanged},
};
use crate::{Control, Server, faults::Proxy};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_control::Error;
use meshway_model::{KeyAlgorithm, NodePublicKey, Version};
use meshway_protocol::{
    EnrollmentResponse, NetmapKeyRotation, ServerSigningKey, SyncNetmap, SyncResponse,
    netmap_key_rotation_transcript,
};
use std::fs;

fn key(seed: u8, generation: u64) -> (SigningKey, ServerSigningKey) {
    let secret = SigningKey::from_bytes(&[seed; 32]);
    let public = ServerSigningKey {
        key_id: format!("00000000-0000-4009-8000-0000000000{seed:02x}")
            .parse()
            .unwrap(),
        key_generation: Version::new(generation).unwrap(),
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: secret.verifying_key().to_bytes().into(),
        },
    };
    (secret, public)
}

/// The fixture key's generation-2 successor.
fn successor() -> (SigningKey, ServerSigningKey) {
    key(44, 2)
}

fn now_ms() -> u64 {
    meshway_test_server::now_ms()
}

/// `response` re-signed by `from`, carrying the handover from `from` to `to`
/// endorsed by `endorser`.
fn handover(
    binding: &EnrollmentResponse,
    mut response: SyncResponse,
    from: &(SigningKey, ServerSigningKey),
    to: &(SigningKey, ServerSigningKey),
    endorser: &SigningKey,
    not_before_ms: u64,
) -> SyncResponse {
    response.signature = sign_with(&from.0, &from.1, binding, &response.lease);
    let message = netmap_key_rotation_transcript(
        binding.network_id,
        from.1.key_id,
        from.1.key_generation,
        &to.1,
        not_before_ms,
    );
    response.key_rotation = Some(NetmapKeyRotation {
        next: to.1.clone(),
        not_before_ms,
        endorsement: URL_SAFE_NO_PAD.encode(endorser.sign(&message).to_bytes()),
        lease_signature: sign_with(&to.0, &to.1, binding, &response.lease),
    });
    response
}

/// What the Server answers during the window: signed by the fixture key,
/// handing over to [`successor`].
fn rotating(binding: &EnrollmentResponse, netmap: &CompiledNetmap) -> SyncResponse {
    let old = server_key();
    handover(
        binding,
        unchanged(binding, netmap, HOUR),
        &old,
        &successor(),
        &old.0,
        now_ms() - 1,
    )
}

/// What the Server answers after the window: signed by the successor only.
fn after_window(binding: &EnrollmentResponse, netmap: &CompiledNetmap) -> SyncResponse {
    let (secret, key) = successor();
    let mut response = unchanged(binding, netmap, HOUR);
    response.signature = sign_with(&secret, &key, binding, &response.lease);
    response
}

fn pinned(control: &Control) -> ServerSigningKey {
    control.status().binding.unwrap().server_signing_key
}

async fn first_netmap(
    server: &Server,
    proxy: &Proxy,
    name: &str,
) -> (
    Control,
    EnrollmentResponse,
    std::sync::Arc<meshway_test_server::Script>,
    CompiledNetmap,
) {
    let (mut control, binding, script) = enrolled(server, proxy, name).await;
    let netmap = compiled(&binding, name, None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    (control, binding, script, netmap)
}

#[tokio::test]
async fn an_endorsed_successor_is_pinned_and_then_only_its_signatures_verify() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script, netmap) = first_netmap(&server, &proxy, "pin").await;
    assert_eq!(
        script.requests()[0].server_key_generation,
        Some(Version::MIN)
    );

    script.push(Ok(rotating(&binding, &netmap)));
    let renewed = control.sync(&mut Facts::default()).await.unwrap();
    let (_, next) = successor();
    assert_eq!(pinned(&control), next);
    assert_eq!(control.netmap().unwrap().lease, renewed.netmap.lease);
    // Both files follow the new pin: the binding and the cached lease signature.
    let path = server.path("pin");
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("control.json")).unwrap()).unwrap();
    assert_eq!(
        record["binding"]["serverSigningKey"],
        serde_json::to_value(&next).unwrap()
    );
    let cache: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("netmap.json")).unwrap()).unwrap();
    assert_eq!(cache["signature"]["keyId"], next.key_id.to_string());
    assert_eq!(cache["signature"]["keyGeneration"], 2);

    // The same handover again is idempotent: the successor's signature verifies.
    script.push(Ok(rotating(&binding, &netmap)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(pinned(&control), next);
    assert_eq!(
        script.requests().last().unwrap().server_key_generation,
        Some(Version::new(2).unwrap())
    );

    // The replaced key alone no longer verifies; the held lease stays.
    let held = control.netmap_lease();
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    assert!(matches!(
        control.sync(&mut Facts::default()).await.err().unwrap(),
        Error::ServerKeyMismatch
    ));
    assert_eq!(control.netmap_lease(), held);
    // After the window the successor signs alone.
    script.push(Ok(after_window(&binding, &netmap)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert!(control.netmap_lease().unwrap().expires_at_ms >= held.unwrap().expires_at_ms);
    control.shutdown().await.unwrap();

    // The rewritten cache is authority under the new pin without the Server.
    let restored = Control::open(
        &path,
        &proxy.origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert_eq!(pinned(&restored), next);
    assert!(restored.netmap().is_some());
    restored.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn skipped_forged_or_premature_handovers_are_not_followed() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script, netmap) = first_netmap(&server, &proxy, "forged").await;
    let old = server_key();
    let held = control.netmap_lease();
    let rejects = async |control: &mut Control, response: SyncResponse| {
        script.push(Ok(response));
        control.sync(&mut Facts::default()).await.err().unwrap()
    };

    // Endorsed by another key than the pinned one.
    let forged = handover(
        &binding,
        unchanged(&binding, &netmap, HOUR),
        &old,
        &successor(),
        &SigningKey::from_bytes(&[45; 32]),
        now_ms() - 1,
    );
    assert!(matches!(
        rejects(&mut control, forged).await,
        Error::SignatureInvalid
    ));
    // Endorsed, but the successor's lease signature is by another key.
    let mut bad_lease = rotating(&binding, &netmap);
    let rotation = bad_lease.key_rotation.as_mut().unwrap();
    rotation.lease_signature = sign_with(&old.0, &rotation.next, &binding, &bad_lease.lease);
    assert!(matches!(
        rejects(&mut control, bad_lease).await,
        Error::SignatureInvalid
    ));
    // A handover between two later generations skips the pinned one.
    let (two, three) = (successor(), key(46, 3));
    let skipped = handover(
        &binding,
        unchanged(&binding, &netmap, HOUR),
        &two,
        &three,
        &two.0,
        now_ms() - 1,
    );
    assert!(matches!(
        rejects(&mut control, skipped).await,
        Error::ServerKeyMismatch
    ));
    // A successor two generations ahead of the pinned key is inconsistent.
    let jump = handover(
        &binding,
        unchanged(&binding, &netmap, HOUR),
        &old,
        &three,
        &old.0,
        now_ms() - 1,
    );
    assert!(matches!(rejects(&mut control, jump).await, Error::Contract));
    assert_eq!(pinned(&control), old.1);
    assert_eq!(
        control.netmap_lease(),
        held,
        "nothing rejected changed the lease"
    );

    // Not valid yet: the answer verifies under the pinned key, the pin stays.
    let premature = handover(
        &binding,
        unchanged(&binding, &netmap, HOUR),
        &old,
        &successor(),
        &old.0,
        now_ms() + HOUR,
    );
    script.push(Ok(premature));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(pinned(&control), old.1);
    // A new content full answer during the window both installs and pins.
    let next = compiled(&binding, "forged-two", Some(&netmap));
    let response = signed(
        &binding,
        meshway_test_server::lease(next.reference(), HOUR),
        SyncNetmap::Full {
            netmap: Box::new(next.clone()),
        },
    );
    script.push(Ok(handover(
        &binding,
        response,
        &old,
        &successor(),
        &old.0,
        now_ms() - 1,
    )));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(control.netmap_reference(), Some(next.reference()));
    assert_eq!(pinned(&control), successor().1);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn a_node_that_missed_the_window_keeps_its_lease_and_rejects_the_successor() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script, netmap) = first_netmap(&server, &proxy, "missed").await;
    let held = control.netmap().unwrap();
    script.push(Ok(after_window(&binding, &netmap)));
    let error = control.sync(&mut Facts::default()).await.err().unwrap();
    assert!(matches!(error, Error::ServerKeyMismatch));
    assert_eq!(error.recovery(), meshway_control::Recovery::Halt);
    assert_eq!(pinned(&control), server_key().1);
    assert_eq!(control.netmap().unwrap().lease, held.lease, "not withdrawn");
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
