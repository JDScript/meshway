pub mod netmap;
pub mod packet;
pub mod secure;

use meshway_control::Control;
use meshway_model::{NodeId, RequestId};
use meshway_test_server::TestServer;
use std::{collections::BTreeMap, time::Duration};

pub use meshway_test_server::Owner;

pub fn request() -> RequestId {
    meshway_test_server::random_id()
}

/// Node traffic goes through the pinning proxy so enrollment pins the
/// fixture netmap key.
pub struct Server(TestServer);

pub struct Node {
    pub id: NodeId,
    pub control: Control,
    /// Bindings committed by enrollment.
    pub keys: meshway_protocol::NodeKeyMetadata,
}

impl Node {
    /// Control's currently published netmap as desired content.
    pub fn desired(&self) -> meshway_engine::Desired {
        meshway_engine::Desired::new(&self.control.netmap().unwrap().content).unwrap()
    }

    pub fn packet_keys(&self) -> meshway_authority::DataPlaneKeys {
        self.control.dataplane_keys().unwrap()
    }
}

impl Server {
    pub async fn start() -> Self {
        Self(TestServer::start().await)
    }

    pub async fn owner(&self) -> Owner {
        self.0
            .owner(
                "runtime-owner@example.test",
                "real runtime fixture password",
                "Runtime HTTP fixture",
            )
            .await
    }

    pub async fn enroll_with_capabilities(
        &self,
        owner: &Owner,
        hostname: &str,
        capabilities: BTreeMap<String, meshway_model::state::Capability>,
    ) -> Node {
        let mut control = self.0.open_control(
            hostname,
            meshway_test_server::http().timeout(Duration::from_secs(10)),
        );
        assert!(control.netmap().is_none());
        let enrolled = self.0.enroll(owner, &mut control, hostname).await;
        control.authenticate().await.unwrap();
        assert!(control.netmap().is_none());
        control.acquire_instance(capabilities).await.unwrap();
        assert!(control.netmap().is_none());
        Node {
            id: enrolled.node_id,
            control,
            keys: enrolled.keys,
        }
    }

    pub async fn close(self) {
        self.0.close().await;
    }
}
