use meshway_control_client::Client;
use meshway_server::{App, Settings, http};
use std::time::Duration;

pub struct Server(pub tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn serve(router: axum::Router, budget: usize) -> (Client, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = Client::new(
        &base,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy()
            .timeout(Duration::from_secs(5)),
        budget,
    )
    .unwrap();
    (client, Server(task))
}

pub async fn controller(registration: bool) -> (App, Client, Server, tempfile::TempDir) {
    let directory = tempfile::Builder::new()
        .prefix("meshway-sdk-")
        .tempdir()
        .unwrap();
    let mut settings = Settings::new(directory.path().canonicalize().unwrap().join("instance"));
    settings.registration_enabled = registration;
    let app = App::open(settings).await.unwrap();
    let (client, server) = serve(http::router(&app), 1024 * 1024).await;
    (app, client, server, directory)
}
