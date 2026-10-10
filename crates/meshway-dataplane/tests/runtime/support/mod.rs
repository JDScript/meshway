pub mod netmap;
pub mod packet;
pub mod secure;

use meshway_control::Control;
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client};
use meshway_model::{NetworkId, NodeId, RequestId};
use meshway_protocol::{
    ApproveEnrollmentRequest, CreateNetworkRequest, RegisterRequest, WireSecret,
};
use meshway_server::{App, Settings};
use std::{collections::BTreeMap, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

pub fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}

pub struct Server {
    /// Node traffic goes through it so enrollment pins the fixture netmap key.
    proxy: meshway_netmap_fixture::PinningProxy,
    client: Client,
    app: App,
    temp: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

pub struct Owner {
    token: WireSecret,
    network: NetworkId,
}

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
        let temp = tempfile::tempdir().unwrap();
        let app = App::open(Settings::new(
            temp.path().canonicalize().unwrap().join("server"),
        ))
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
        let (stop, done) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = done.await;
                })
                .await
                .unwrap();
        });
        let client = Client::new(
            &origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
            16 * 1024 * 1024,
        )
        .unwrap();
        let proxy = meshway_netmap_fixture::PinningProxy::start(&origin).await;
        Self {
            proxy,
            client,
            app,
            temp,
            stop,
            task,
        }
    }

    pub async fn owner(&self) -> Owner {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "runtime-owner@example.test".into(),
                password: WireSecret::new("real runtime fixture password".into()),
            })
            .await
            .unwrap();
        let response = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "Runtime HTTP fixture".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        Owner {
            token: session.token,
            network: response.network.id,
        }
    }

    pub async fn enroll_with_capabilities(
        &self,
        owner: &Owner,
        hostname: &str,
        capabilities: BTreeMap<String, meshway_model::state::Capability>,
    ) -> Node {
        let path = self.temp.path().canonicalize().unwrap().join(hostname);
        let mut control = Control::open(
            &path,
            &self.proxy.origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
        )
        .unwrap();
        assert!(control.netmap().is_none());
        let pending = control.begin_enrollment(hostname).await.unwrap();
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: owner.network,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        let enrolled = control.redeem_enrollment().await.unwrap();
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
        self.proxy.close().await;
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("bounded HTTP server shutdown")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.app.shutdown())
            .await
            .expect("bounded database owner shutdown")
            .unwrap();
    }
}
