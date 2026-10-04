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
