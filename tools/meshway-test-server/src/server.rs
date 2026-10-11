//! A real Server in-process: temporary data directory, random loopback port,
//! the generated client and production `Control` enrollment.
use crate::proxy::PinningProxy;
use meshway_control::Control;
use meshway_control_client::{ApproveDeviceAuthorizationPath, Client, ClientError};
use meshway_model::{NetworkId, state::Capability};
use meshway_protocol::{
    ApproveEnrollmentRequest, CreateNetworkRequest, EnrollmentResponse, ErrorResponse,
    RegisterRequest, SessionIssuedResponse, WireSecret,
};
use meshway_server::{App, Settings};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

/// Response budget of [`TestServer::client`].
pub const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

/// A uniformly random identifier such as `RequestId` or a subscription ID.
pub fn random_id<T: TryFrom<uuid::Uuid>>() -> T
where
    T::Error: std::fmt::Debug,
{
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).expect("random identifier");
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .try_into()
        .expect("random v4 identifier")
}

/// The production HTTP builder (platform TLS) without system proxies.
pub fn http() -> reqwest::ClientBuilder {
    meshway_control_client::http_client_builder()
        .expect("TLS configuration")
        .no_proxy()
}

/// Register a fixture user while respecting the Server's process-wide KDF
/// capacity. Only a busy response is retried, within a bounded setup budget.
pub async fn register(
    client: &Client,
    request: &RegisterRequest,
) -> Result<SessionIssuedResponse, ClientError<ErrorResponse>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match client.register(request).await {
            Err(ClientError::Http {
                status: 503,
                problem: Some(ref body),
            }) if matches!(body.problem, meshway_protocol::Problem::Busy { .. })
                && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

/// How [`TestServer::start_with`] configures the Server.
#[derive(Clone, Debug)]
pub struct Options {
    /// Hold of an unchanged sync; `None` keeps the Server default.
    pub hold: Option<Duration>,
    /// Route Node traffic through a [`PinningProxy`] so enrollment pins the
    /// fixture netmap key and scripted replies verify.
    pub pin: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hold: None,
            pin: true,
        }
    }
}

/// A registered user session and the network it created.
pub struct Owner {
    pub token: WireSecret,
    pub network: NetworkId,
}

pub struct TestServer {
    /// The real Server.
    pub origin: String,
    /// Where Nodes connect: the pinning proxy, or `origin` without one.
    pub node_origin: String,
    /// Generated client against `origin` (10 s timeout).
    pub client: Client,
    proxy: Option<PinningProxy>,
    app: App,
    temp: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl TestServer {
    /// A Server behind the key-pinning proxy, default hold.
    pub async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    pub async fn start_with(options: Options) -> Self {
        let temp = tempfile::tempdir().expect("temporary directory");
        let mut settings = Settings::new(
            temp.path()
                .canonicalize()
                .expect("temporary path")
                .join("server"),
        );
        if let Some(hold) = options.hold {
            settings.netmap.hold = hold;
        }
        let app = App::open(settings).await.expect("open Server");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Server listener");
        let origin = format!("http://{}", listener.local_addr().expect("Server address"));
        let router = meshway_server::http::router(&app);
        let (stop, done) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = done.await;
                })
                .await
                .expect("Server serve");
        });
        let client = Client::new(
            &origin,
            http().timeout(Duration::from_secs(10)),
            RESPONSE_LIMIT,
        )
        .expect("generated client");
        let proxy = if options.pin {
            Some(PinningProxy::start(&origin).await)
        } else {
            None
        };
        Self {
            node_origin: proxy
                .as_ref()
                .map_or_else(|| origin.clone(), |proxy| proxy.origin.clone()),
            origin,
            client,
            proxy,
            app,
            temp,
            stop,
            task,
        }
    }

    /// The pinning proxy, when started with one.
    pub fn proxy(&self) -> Option<&PinningProxy> {
        self.proxy.as_ref()
    }

    /// A private path in the fixture's temporary directory.
    pub fn path(&self, name: &str) -> PathBuf {
        self.temp
            .path()
            .canonicalize()
            .expect("temporary path")
            .join(name)
    }

    /// Register `email` and create one network.
    pub async fn owner(&self, email: &str, password: &str, network: &str) -> Owner {
        let session = register(
            &self.client,
            &RegisterRequest {
                email: email.into(),
                password: WireSecret::new(password.into()),
            },
        )
        .await
        .expect("register owner");
        let response = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: random_id(),
                    name: network.into(),
                },
                session.token.expose(),
            )
            .await
            .expect("create network");
        Owner {
            token: session.token,
            network: response.network.id,
        }
    }

    /// A production Control stored at [`Self::path`]`(name)`, connecting to
    /// [`Self::node_origin`].
    pub fn open_control(&self, name: &str, http: reqwest::ClientBuilder) -> Control {
        Control::open(&self.path(name), &self.node_origin, http).expect("open Control")
    }

    /// Device-authorize `control` into `network` by `token` and redeem it.
    pub async fn enroll_into(
        &self,
        token: &WireSecret,
        network: NetworkId,
        control: &mut Control,
        hostname: &str,
    ) -> EnrollmentResponse {
        let pending = control
            .begin_enrollment(hostname)
            .await
            .expect("begin enrollment");
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                token.expose(),
            )
            .await
            .expect("approve enrollment");
        control.poll_enrollment().await.expect("poll enrollment");
        control
            .redeem_enrollment()
            .await
            .expect("redeem enrollment")
    }

    /// Device-authorize `control` into the owner's network and redeem it.
    pub async fn enroll(
        &self,
        owner: &Owner,
        control: &mut Control,
        hostname: &str,
    ) -> EnrollmentResponse {
        self.enroll_into(&owner.token, owner.network, control, hostname)
            .await
    }

    /// Enroll, authenticate and acquire an instance with `capabilities`.
    pub async fn connected(
        &self,
        owner: &Owner,
        control: &mut Control,
        hostname: &str,
        capabilities: BTreeMap<String, Capability>,
    ) -> EnrollmentResponse {
        let binding = self.enroll(owner, control, hostname).await;
        control.authenticate().await.expect("authenticate");
        control
            .acquire_instance(capabilities)
            .await
            .expect("acquire instance");
        binding
    }

    /// Start a netmap signing-key rotation window on the running Server, as
    /// `meshway-server rotate-netmap-key` does on stopped storage.
    pub async fn rotate_netmap_key(
        &self,
        window: Duration,
    ) -> meshway_server::netmap::signing::RotatedSigningKey {
        self.app
            .rotate_netmap_key(window)
            .await
            .expect("rotate netmap signing key")
    }

    /// Bounded shutdown of the proxy, HTTP server and database owner.
    pub async fn close(self) {
        if let Some(proxy) = self.proxy {
            proxy.close().await;
        }
        self.app.stream_shutdown()();
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("bounded HTTP server shutdown")
            .expect("HTTP server task");
        tokio::time::timeout(Duration::from_secs(10), self.app.shutdown())
            .await
            .expect("bounded database owner shutdown")
            .expect("database owner");
    }
}
