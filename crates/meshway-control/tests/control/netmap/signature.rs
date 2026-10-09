//! Only responses signed by the Server key pinned at enrollment change the
//! netmap authority; a rejected response changes nothing and revokes nothing.
use super::{
    HOUR, enrolled,
    support::{Facts, Script, compiled, full, server_key, sign, sign_with},
};
use crate::{Control, Server, faults::Proxy};
use ed25519_dalek::SigningKey;
use meshway_control::Error;
use meshway_model::{Digest, Version};
use meshway_protocol::SyncResponse;

async fn rejects(control: &mut Control, script: &Script, response: SyncResponse) -> Error {
    script.push(Ok(response));
    control.sync(&mut Facts::default()).await.err().unwrap()
}

#[tokio::test]
async fn tampered_digest_lease_signature_key_or_node_are_rejected_without_state_change() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "tamper").await;
    let first = compiled(&binding, "tamper", None);
    script.push(Ok(full(&binding, &first, HOUR)));
    let current = control.sync(&mut Facts::default()).await.unwrap().snapshot;
    let held = control.netmap_lease();
    let next = compiled(&binding, "tamper-two", Some(&first));

    // Signed over another digest than the delivered content and lease.
    let mut response = full(&binding, &next, HOUR);
    let mut other = response.lease;
    other.netmap.digest = Digest::from_bytes([9; 32]);
    response.signature = sign(&binding, &other);
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));
    // Lease changed after signing.
    let mut response = full(&binding, &next, HOUR);
    response.lease.expires_at_ms += 1_000;
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));
    // Signature bytes altered.
    let mut response = full(&binding, &next, HOUR);
    let text = response.signature.signature.clone();
    let flipped = if text.starts_with('A') { "B" } else { "A" };
    response.signature.signature = format!("{flipped}{}", &text[1..]);
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));
    // Another key ID or generation than the pin.
    let mut response = full(&binding, &next, HOUR);
    response.signature.key_id = crate::support::request().to_string().parse().unwrap();
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::ServerKeyMismatch
    ));
    let mut response = full(&binding, &next, HOUR);
    response.signature.key_generation = Version::new(2).unwrap();
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::ServerKeyMismatch
    ));
    // The pinned key ID, but another private key.
    let (_, pinned) = server_key();
    let mut response = full(&binding, &next, HOUR);
    response.signature = sign_with(
        &SigningKey::from_bytes(&[43; 32]),
        &pinned,
        &binding,
        &response.lease,
    );
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));
    // A signature the Server made for another Node.
    let mut response = full(&binding, &next, HOUR);
    let mut stranger = binding.clone();
    stranger.node_id = crate::support::request().to_string().parse().unwrap();
    response.signature = sign(&stranger, &response.lease);
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));

    // The verified authority is untouched and still renews normally.
    current.ensure_current().unwrap();
    assert_eq!(control.netmap_lease(), held);
    assert_eq!(control.netmap_reference(), Some(first.reference()));
    assert!(control.netmap_snapshot().unwrap().same_netmap(&current));
    script.push(Ok(full(&binding, &next, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(control.netmap_reference(), Some(next.reference()));
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn unpinned_binding_accepts_no_netmap() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut control = Control::open(
        &server.path("unpinned"),
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let binding = server.enroll(&owner, &mut control, "unpinned").await;
    assert_eq!(binding.server_signing_key, None);
    control.authenticate().await.unwrap();
    let script = Script::new();
    control.set_sync_transport(script.clone());
    let netmap = compiled(&binding, "unpinned", None);
    let response = full(&binding, &netmap, HOUR);
    assert!(matches!(
        rejects(&mut control, &script, response).await,
        Error::SignatureInvalid
    ));
    assert!(control.netmap_snapshot().is_err());
    assert_eq!(control.netmap_reference(), None);
    assert!(!server.path("unpinned").join("netmap.json").exists());
    control.shutdown().await.unwrap();
    server.close().await;
}
