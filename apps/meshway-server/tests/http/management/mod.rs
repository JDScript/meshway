use super::*;
use meshway_model::{MembershipRole, RequestId, Revision};
use meshway_protocol::{
    CreateNetworkRequest, NetworkNameRules, NetworkResponse, configuration::*, history::*,
    management::*,
};

pub(super) fn request() -> RequestId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    RequestId::new(uuid::Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    )))
    .unwrap()
}

pub(super) async fn source(
    c: &Client,
    s: &Server,
    network: meshway_model::NetworkId,
    token: &str,
) -> ConfigurationSourceResponse {
    c.get(net(s, network, "configuration/source"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

pub(super) async fn create(c: &Client, s: &Server, token: &str, name: &str) -> NetworkResponse {
    let response = c
        .post(format!("{}/api/v1/networks", s.url))
        .bearer_auth(token)
        .json(&CreateNetworkRequest {
            request_id: request(),
            name: name.into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json().await.unwrap()
}
pub(super) fn net(s: &Server, n: meshway_model::NetworkId, path: &str) -> String {
    format!("{}/api/v1/networks/{n}/{path}", s.url)
}
fn query(url: String, params: &[(&str, &str)]) -> reqwest::Url {
    let mut url = reqwest::Url::parse(&url).unwrap();
    url.query_pairs_mut().extend_pairs(params.iter().copied());
    url
}

pub(super) async fn join(
    c: &Client,
    s: &Server,
    network: meshway_model::NetworkId,
    issuer: &str,
    recipient: &str,
    role: MembershipRole,
) -> meshway_model::MembershipId {
    let created: JoinLinkCreated = json(
        c.post(net(s, network, "join-links"))
            .bearer_auth(issuer)
            .json(&CreateJoinLinkRequest {
                request_id: request(),
                role,
                expires_in_seconds: 600,
            }),
    )
    .await;
    let JoinLinkCreated::Available { secret, .. } = created else {
        panic!("missing first delivery")
    };
    c.post(format!("{}/api/v1/join-links/redeem", s.url))
        .bearer_auth(recipient)
        .json(&JoinLinkCredentialRequest {
            token: WireSecret::new(secret.expose().into()),
        })
        .send()
        .await
        .unwrap()
        .json::<JoinLinkRedeemed>()
        .await
        .unwrap()
        .membership_id
}

mod account;

mod configuration;

mod delegation;

mod identity;

mod members;
