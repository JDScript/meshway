//! Single-node HTTP/Control setup for DNS lifecycle tests; no packet runtime.
pub mod dns;

use meshway_control::Control;
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery,
};
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, RegisterRequest,
    WireSecret,
};
use meshway_server::{App, Settings};
use std::{collections::BTreeMap, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

pub const WAIT: Duration = Duration::from_secs(1);

pub fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}

pub struct Node {
    pub control: Control,
}

impl Node {
    pub async fn sync_current(&mut self) -> meshway_authority::ExecutionSnapshot {
        let target = self.control.sync_configuration().await.unwrap().unwrap();
        assert_eq!(self.control.status().cached, Some(target.clone()));
        if self
            .control
            .status()
            .instance
            .unwrap()
            .execution_expires_at_ms
            .is_none()
        {
            assert!(matches!(
                self.control.execution_snapshot(),
                Err(meshway_control::Error::Expired)
            ));
        }
        let lease = self.control.renew_instance().await.unwrap();
        assert_eq!(lease.target, Some(target.clone()));
        let snapshot = self.control.execution_snapshot().unwrap();
        assert_eq!(snapshot.reference(), target);
        snapshot
    }
}

pub struct Server {
    origin: String,
    client: Client,
    token: WireSecret,
    network: NetworkId,
    app: App,
    temp: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
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
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
            16 * 1024 * 1024,
        )
        .unwrap();
        let session = client
            .register(&RegisterRequest {
                email: "dns-lifecycle-owner@example.test".into(),
                password: WireSecret::new("real DNS lifecycle fixture password".into()),
            })
            .await
            .unwrap();
        let network = client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "DNS Control lifecycle fixture".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        Self {
            origin,
            client,
            token: session.token,
            network,
            app,
            temp,
            stop,
            task,
        }
    }

    pub async fn enroll(&self, hostname: &str) -> Node {
        use meshway_model::state::{Capability, StreamKind, TransportRole};
        let path = self.temp.path().canonicalize().unwrap().join(hostname);
        let mut control = Control::open(
            &path,
            &self.origin,
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
        )
        .unwrap();
        assert!(control.execution_snapshot().is_err());
        let pending = control.begin_enrollment(hostname).await.unwrap();
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: self.network,
                },
                self.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        control.redeem_enrollment().await.unwrap();
        control.authenticate().await.unwrap();
        assert!(control.execution_snapshot().is_err());
        control
            .acquire_instance(BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]))
            .await
            .unwrap();
        assert!(control.execution_snapshot().is_err());
        Node { control }
    }

    pub async fn deny_access(&self) {
        let source = self
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: self.network,
                },
                &GetConfigurationSourceQuery::default(),
                self.token.expose(),
            )
            .await
            .unwrap();
        let mut input =
            meshway_configuration::NetworkConfigurationInput::from(&source.configuration);
        input.access.clear();
        self.client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: self.network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: request(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
                self.token.expose(),
            )
            .await
            .unwrap();
    }

    pub async fn close(self) {
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
