use meshway_control_client::Client;
use meshway_ipc::{Command as IpcCommand, Phase, Request, Response, ResultBody, Status, VERSION};
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::{CreateNetworkRequest, RegisterRequest, WireSecret};
use meshway_server::{App, Settings};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{net::UnixStream, sync::oneshot, task::JoinHandle, time::Instant};

pub fn request_id() -> RequestId {
    let mut b = [0; 16];
    getrandom::fill(&mut b).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(b).into_uuid()).unwrap()
}
pub struct Server {
    pub origin: String,
    pub client: Client,
    app: App,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}
impl Server {
    pub async fn start(path: PathBuf) -> Self {
        let app = App::open(Settings::new(path)).await.unwrap();
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
            stop,
            task,
        }
    }
    pub async fn owner(&self) -> (WireSecret, NetworkId) {
        let session = self
            .client
            .register(&RegisterRequest {
                email: "daemon@example.test".into(),
                password: WireSecret::new("daemon test password".into()),
            })
            .await
            .unwrap();
        let network = self
            .client
            .create_network(
                &CreateNetworkRequest {
                    request_id: request_id(),
                    name: "Daemon lifecycle".into(),
                },
                session.token.expose(),
            )
            .await
            .unwrap();
        (session.token, network.network.id)
    }
    pub async fn close(self) {
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}
pub struct Process {
    pub child: Child,
    pub socket: PathBuf,
}
impl Process {
    pub fn start(path: &Path, origin: &str) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_meshwayd"))
            .args([
                "--server",
                origin,
                "--hostname",
                "daemon-node",
                "--console-origin",
                "http://127.0.0.1:3001",
            ])
            .arg("--data-dir")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            child,
            socket: path.join("control.sock"),
        }
    }
    pub async fn exit(&mut self, signal: &str) {
        assert!(
            Command::new("kill")
                .args([signal, &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "daemon failed to stop");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub async fn call(path: &Path, command: IpcCommand, id: RequestId) -> Result<Response, ()> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut stream = UnixStream::connect(path).await.map_err(|_| ())?;
        meshway_ipc::write(
            &mut stream,
            &Request {
                version: VERSION,
                request_id: id,
                command,
            },
        )
        .await
        .map_err(|_| ())?;
        let response: Response = meshway_ipc::read(&mut stream).await.map_err(|_| ())?;
        assert_eq!(response.request_id, id);
        Ok(response)
    })
    .await
    .map_err(|_| ())?
}
pub async fn status(path: &Path) -> Result<Status, ()> {
    match call(path, IpcCommand::Status, request_id()).await?.result {
        ResultBody::Status { status } => Ok(*status),
        _ => Err(()),
    }
}
pub async fn wait(path: &Path, phase: Phase) -> Status {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(status) = status(path).await {
            if status.phase == phase {
                return status;
            }
            assert_ne!(status.phase, Phase::Blocked, "{:?}", status.last_error);
        }
        assert!(
            Instant::now() < deadline,
            "daemon phase did not converge to {phase:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
