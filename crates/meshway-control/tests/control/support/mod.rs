use meshway_control_client::Client;
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{CreateNetworkRequest, RegisterRequest, WireSecret};
use meshway_server::{App, Settings};
use std::{path::PathBuf, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

pub struct Server {
    pub origin: String,
    pub client: Client,
    app: App,
    temp: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}
pub struct Owner {
    pub token: WireSecret,
    pub network: NetworkId,
}
pub fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}
impl Server {
    pub async fn control(
        &self,
        owner: &Owner,
        origin: &str,
        name: &str,
        capabilities: std::collections::BTreeMap<String, meshway_model::state::Capability>,
    ) -> meshway_control::Control {
        let mut control = meshway_control::Control::open(
            &self.path(name),
            origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy(),
        )
        .unwrap();
        self.enroll(owner, &mut control, name).await;
        control.authenticate().await.unwrap();
        control.acquire_instance(capabilities).await.unwrap();
        control
    }

    pub async fn enroll(
        &self,
        owner: &Owner,
        control: &mut meshway_control::Control,
        hostname: &str,
    ) -> meshway_protocol::EnrollmentResponse {
        let pending = control.begin_enrollment(hostname).await.unwrap();
        self.client
            .approve_device_authorization(
                &meshway_control_client::ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &meshway_protocol::ApproveEnrollmentRequest {
                    network_id: owner.network,
                },
                owner.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        control.redeem_enrollment().await.unwrap()
    }
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::new(temp.path().canonicalize().unwrap().join("server"));
        // An unchanged sync answers after one second instead of a full hold.
        settings.netmap.hold = Duration::from_secs(1);
        let app = App::open(settings).await.unwrap();
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
        Self {
            origin,
            client,
            app,
            temp,
            stop,
            task,
        }
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.temp.path().canonicalize().unwrap().join(name)
    }
    pub async fn owner(&self) -> Owner {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "node-control@example.test".into(),
                password: WireSecret::new("real control fixture password".into()),
            })
            .await
            .unwrap();
        let network = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request(),
                    name: "Control fixture".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        Owner {
            token: session.token,
            network: network.network.id,
        }
    }
    pub async fn close(self) {
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}

/// Peer facts are irrelevant to these Control fixtures.
#[derive(Default)]
pub struct NoFacts;
impl meshway_control::PeerFactsSink for NoFacts {
    fn apply(
        &mut self,
        _: meshway_model::NetmapRef,
        _: meshway_protocol::PeerFactsDelta,
    ) -> Result<(), meshway_control::FactsRejected> {
        Ok(())
    }
}

/// One real `sync` exchange; returns the netmap Control now holds.
pub async fn sync(
    control: &mut meshway_control::Control,
) -> Result<Option<meshway_model::NetmapRef>, meshway_control::Error> {
    control
        .sync(&mut NoFacts)
        .await
        .map(|outcome| Some(outcome.netmap.content.reference()))
}

/// Save receipt of the held netmap for the current instance.
pub async fn acknowledge_saved(
    control: &mut meshway_control::Control,
) -> Result<meshway_control::ApplicationTicket, meshway_control::Error> {
    let netmap = control
        .netmap_reference()
        .ok_or(meshway_control::Error::Lifecycle)?;
    control.begin_application(netmap).await
}

/// An enrolled, authenticated Control with a Native UDP capable instance.
pub async fn udp_control(
    server: &Server,
    owner: &Owner,
    origin: &str,
    name: &str,
) -> meshway_control::Control {
    use meshway_model::state::{Capability, StreamKind, TransportRole};
    server
        .control(
            owner,
            origin,
            name,
            std::collections::BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]),
        )
        .await
}
