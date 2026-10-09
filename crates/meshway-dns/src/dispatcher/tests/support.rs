//! Private assembly only: enrollment, configuration and execution all use the
//! generated client and production Server/Control implementations.
use meshway_configuration::{EgressRule, NetworkConfigurationInput, RouteMatch, RuleAction};
use meshway_control::Control;
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery,
};
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, RegisterRequest,
    WireSecret,
};
use meshway_server::{App, Settings};
use std::{
    future::{Future, poll_fn},
    panic::AssertUnwindSafe,
    task::Poll,
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinSet};
use tokio_util::sync::CancellationToken;

pub(super) struct Server {
    app: Option<App>,
    tasks: JoinSet<()>,
    stop: CancellationToken,
    client: Client,
    /// Node traffic goes through it so enrollment pins the fixture netmap key.
    proxy: meshway_netmap_fixture::PinningProxy,
    temporary: tempfile::TempDir,
}

impl Server {
    pub(super) async fn start() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let app = App::open(Settings::new(
            temporary.path().canonicalize().unwrap().join("server"),
        ))
        .await
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let client = Client::new(
            &origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy()
                .timeout(Duration::from_secs(10)),
            16 * 1024 * 1024,
        )
        .unwrap();
        let router = meshway_server::http::router(&app);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        let proxy = meshway_netmap_fixture::PinningProxy::start(&origin).await;
        Self {
            app: Some(app),
            tasks,
            stop,
            client,
            proxy,
            temporary,
        }
    }

    pub(super) fn control(&self) -> Control {
        Control::open(
            &self.temporary.path().canonicalize().unwrap().join("node"),
            &self.proxy.origin,
            meshway_control_client::http_client_builder()
                .unwrap()
                .no_proxy(),
        )
        .unwrap()
    }

    pub(super) async fn enroll(&self, control: &mut Control) {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "dispatcher-owner@example.test".into(),
                password: WireSecret::new("isolated dispatcher fixture password".into()),
            })
            .await
            .unwrap();
        let network = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: super::request_id(),
                    name: "Dispatcher regression".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap()
            .network
            .id;
        let pending = control.begin_enrollment("dispatcher-node").await.unwrap();
        self.client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: pending.id,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        control.poll_enrollment().await.unwrap();
        control.redeem_enrollment().await.unwrap();

        let source = self
            .client
            .get_configuration_source(
                &GetConfigurationSourcePath {
                    network_id: network,
                },
                &GetConfigurationSourceQuery::default(),
                session.token.expose(),
            )
            .await
            .unwrap();
        let mut input = NetworkConfigurationInput::from(&source.configuration);
        input.dns.override_local = false;
        input.dns.servers.clear();
        input.dns.default.clear();
        input.dns.routes.clear();
        input.routing.egress.rules.push(EgressRule {
            remark: "Exact system resolver fixture destinations".into(),
            matcher: RouteMatch {
                ip_cidr: vec!["192.0.2.53/32".into(), "192.0.2.54/32".into()],
                ..Default::default()
            },
            action: RuleAction::Direct,
        });
        self.client
            .apply_configuration(
                &ApplyConfigurationPath {
                    network_id: network,
                },
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: super::request_id(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        control.authenticate().await.unwrap();
    }

    pub(super) async fn close(mut self) {
        self.app.as_ref().unwrap().stream_shutdown()();
        self.stop.cancel();
        let joined = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(task) = self.tasks.join_next().await {
                task?;
            }
            Ok::<_, tokio::task::JoinError>(())
        })
        .await;
        let shutdown =
            tokio::time::timeout(Duration::from_secs(10), self.app.take().unwrap().shutdown())
                .await;
        joined.expect("bounded HTTP fixture join").unwrap();
        shutdown
            .expect("bounded fixture application shutdown")
            .unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
        // JoinSet also aborts its owned task if setup or teardown itself panics.
    }
}

pub(super) async fn catch(future: impl Future<Output = ()>) -> std::thread::Result<()> {
    let mut future = Box::pin(future);
    poll_fn(
        |cx| match std::panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
            Err(panic) => Poll::Ready(Err(panic)),
        },
    )
    .await
}
