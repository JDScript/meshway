use meshway_control_client::Client;
use meshway_model::{NetworkId, RequestId};
use meshway_protocol::ipc::{
    Command as IpcCommand, Phase, Request, Response, ResultBody, Status, VERSION,
};
use meshway_protocol::{CreateNetworkRequest, RegisterRequest, WireSecret};
use meshway_server::{App, Settings};
use std::{
    io::{BufRead, Read},
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
    backend: Backend,
}
enum Backend {
    Embedded {
        app: Box<App>,
        stop: oneshot::Sender<()>,
        task: JoinHandle<()>,
    },
    External(External),
}
struct External {
    child: Child,
    output: Option<std::thread::JoinHandle<()>>,
}
impl Drop for External {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}
impl Server {
    pub async fn start(path: PathBuf) -> Self {
        let console_origin = "http://127.0.0.1:3001";
        if let Some(binary) = std::env::var_os("MESHWAY_TEST_SERVER_BINARY") {
            let child = Command::new(binary)
                .arg("--data-dir")
                .arg(path)
                .args(["--listen", "127.0.0.1:0", "--registration-enabled", "true"])
                .args(["--console-origin", console_origin])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut external = External {
                child,
                output: None,
            };
            let stdout = external.child.stdout.take().unwrap();
            let (ready, address) = oneshot::channel();
            external.output = Some(std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stdout);
                let mut line = String::new();
                reader.by_ref().take(512).read_line(&mut line).unwrap();
                let _ = ready.send(line);
                let _ = std::io::copy(&mut reader, &mut std::io::sink());
            }));
            let line = tokio::time::timeout(Duration::from_secs(10), address)
                .await
                .unwrap()
                .unwrap();
            let address: std::net::SocketAddr = line
                .trim()
                .strip_prefix("Listening on ")
                .expect("Server readiness line")
                .parse()
                .unwrap();
            assert!(address.ip().is_loopback());
            return Self::client(format!("http://{address}"), Backend::External(external));
        }
        let mut settings = Settings::new(path);
        settings.console_origin = Some(meshway_server::console_origin(console_origin).unwrap());
        let app = App::open_with_connector(
            settings,
            std::sync::Arc::new(meshway_server::resources::download::Offline),
        )
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
        Self::client(
            origin,
            Backend::Embedded {
                app: Box::new(app),
                stop,
                task,
            },
        )
    }
    fn client(origin: String, backend: Backend) -> Self {
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
            backend,
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
        match self.backend {
            Backend::Embedded { app, stop, task } => {
                app.stream_shutdown()();
                stop.send(()).unwrap();
                task.await.unwrap();
                app.shutdown().await.unwrap();
            }
            Backend::External(mut external) => {
                assert!(
                    Command::new("kill")
                        .args(["-TERM", &external.child.id().to_string()])
                        .status()
                        .unwrap()
                        .success()
                );
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Some(status) = external.child.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    assert!(Instant::now() < deadline, "Server shutdown deadline");
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        }
    }
}
pub struct Process {
    pub child: Child,
    pub socket: PathBuf,
}
impl Process {
    pub fn start(path: &Path, origin: &str) -> Self {
        Self::start_with_mode(path, origin, "control-only")
    }
    pub fn start_with_mode(path: &Path, origin: &str, mode: &str) -> Self {
        Self::start_with_options(path, origin, mode, &[])
    }
    pub fn start_with_options(path: &Path, origin: &str, mode: &str, options: &[&str]) -> Self {
        let binary = std::env::var_os("MESHWAY_TEST_DAEMON_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_meshwayd").into());
        let child = Command::new(binary)
            .args([
                "--network-mode",
                mode,
                "--log-format",
                "json",
                "--log-filter",
                "info",
                "--server",
                origin,
                "--hostname",
                "daemon-node",
            ])
            .arg("--data-dir")
            .arg(path)
            .args(options)
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
        meshway_protocol::ipc::write(
            &mut stream,
            &Request {
                version: VERSION,
                request_id: id,
                command,
            },
        )
        .await
        .map_err(|_| ())?;
        let response: Response = meshway_protocol::ipc::read(&mut stream)
            .await
            .map_err(|_| ())?;
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
    wait_matching(path, |status| status.phase == phase).await
}
pub async fn wait_matching(path: &Path, matches: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(status) = status(path).await {
            if matches(&status) {
                return status;
            }
            assert_ne!(status.phase, Phase::Blocked, "{:?}", status.last_error);
        }
        assert!(
            Instant::now() < deadline,
            "daemon status did not converge to the required observed state"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
