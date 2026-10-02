//! Shared fixture construction; behavior tests remain beside their owner.
pub(crate) mod fixtures;
mod single_network;
pub(crate) use single_network::SingleNetworkFixture;
pub(crate) use single_network::{LEGACY_NODE_SELECT, original_columns};

use meshway_protocol::{EnrollAgentRequest, NodeSummary, local};

pub(crate) fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

pub(crate) fn request(name: &str) -> EnrollAgentRequest {
    EnrollAgentRequest {
        node_credential: local::credential().unwrap(),
        name: name.into(),
        agent_version: "v1".into(),
    }
}

pub(crate) fn enroll(
    store: &mut SingleNetworkFixture,
    request: &EnrollAgentRequest,
) -> NodeSummary {
    let token = store.create_token(3600).unwrap();
    store.enroll(&token.token, request).unwrap()
}
