//! Per-request HTTP/1 transport. Every connection is a factory FD, then TLS.
//! No pool, proxy, redirect, unmarked retry, QUIC or ambient system resolver.
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    task::JoinHandle,
    time::{Instant, Sleep},
};
use tokio_rustls::TlsConnector;
use tokio_util::sync::WaitForCancellationFutureOwned;

use super::{Purpose, SocketFactory, closed, dns::BootstrapResolver, invalid};
use meshway_control_client::client::{HttpError, HttpResult, HttpTransport};

trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

pub(crate) struct ControllerHttp {
    tasks: tokio_util::task::TaskTracker,
    factory: SocketFactory,
    resolver: BootstrapResolver,
    origin: reqwest::Url,
    tls: Arc<ClientConfig>,
}

impl ControllerHttp {
    pub(crate) fn system_roots() -> io::Result<RootCertStore> {
        let loaded = rustls_native_certs::load_native_certs();
        let mut roots = RootCertStore::empty();
        for certificate in loaded.certs {
            roots
                .add(certificate)
                .map_err(|error| invalid(error.to_string()))?;
        }
        if roots.is_empty() {
            return Err(io::Error::other("no system TLS trust roots"));
        }
        Ok(roots)
    }

    pub(crate) fn new(
        factory: SocketFactory,
        resolver: BootstrapResolver,
        origin: reqwest::Url,
        roots: RootCertStore,
    ) -> io::Result<Self> {
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(invalid("unsupported controller origin"));
        }
        for variable in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            if std::env::var_os(variable).is_some_and(|value| !value.is_empty()) {
                return Err(invalid(
                    "proxy environment unsupported by protected controller transport",
                ));
            }
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut tls = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| invalid(error.to_string()))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            tasks: tokio_util::task::TaskTracker::new(),
            factory,
            resolver,
            origin,
            tls: Arc::new(tls),
        })
    }

    async fn request(
        &self,
        request: reqwest::Request,
        deadline: Instant,
    ) -> io::Result<reqwest::Response> {
        if let Err(error) = self.resolver.check_system() {
            self.factory.0.retire();
            return Err(error);
        }
        if request.url().origin() != self.origin.origin() {
            return Err(invalid("request escaped controller origin"));
        }
        let hostname = request
            .url()
            .host_str()
            .ok_or_else(|| invalid("missing HTTP host"))?
            .trim_matches(['[', ']']);
        let port = request
            .url()
            .port_or_known_default()
            .ok_or_else(|| invalid("missing HTTP port"))?;
        let addresses = self.resolver.resolve(Purpose::Controller, hostname).await?;
        let mut connected = None;
        let mut last = io::Error::other("controller has no connect candidates");
        for address in addresses {
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                self.factory
                    .tcp(Purpose::Controller, SocketAddr::new(address, port)),
            )
            .await;
            match result {
                Ok(Ok(stream)) => {
                    connected = Some(stream);
                    break;
                }
                Ok(Err(error)) => last = error,
                Err(_) => {
                    last = io::Error::new(io::ErrorKind::TimedOut, "controller candidate timeout")
                }
            }
            if self.factory.0.cancelled.is_cancelled() {
                return Err(closed());
            }
        }
        let tcp = connected.ok_or(last)?;
        let stream: Box<dyn Io> = if request.url().scheme() == "https" {
            let name = ServerName::try_from(hostname.to_owned())
                .map_err(|error| invalid(error.to_string()))?;
            Box::new(
                TlsConnector::from(self.tls.clone())
                    .connect(name, tcp)
                    .await?,
            )
        } else {
            Box::new(tcp)
        };
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
                .await
                .map_err(|error| io::Error::other(error.to_string()))?;
        let connection_task = self.tasks.spawn(async move {
            let _ = connection.await;
        });
        // Abort even if headers fail or this future is cancelled by timeout.
        let mut driver = Driver(Some(connection_task));
        let url = request.url();
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let authority = match url.port() {
            Some(port) => format!(
                "{}:{port}",
                url.host_str().ok_or_else(|| invalid("missing host"))?
            ),
            None => url
                .host_str()
                .ok_or_else(|| invalid("missing host"))?
                .to_owned(),
        };
        let body = request
            .body()
            .map(|body| {
                body.as_bytes()
                    .ok_or_else(|| invalid("streaming controller request unsupported"))
            })
            .transpose()?
            .unwrap_or_default();
        let mut outgoing = http::Request::builder()
            .method(request.method().clone())
            .uri(path)
            .body(Full::new(Bytes::copy_from_slice(body)))
            .map_err(|error| invalid(error.to_string()))?;
        *outgoing.headers_mut() = request.headers().clone();
        outgoing.headers_mut().insert(
            http::header::HOST,
            authority
                .parse()
                .map_err(|error: http::header::InvalidHeaderValue| invalid(error.to_string()))?,
        );
        let response = sender
            .send_request(outgoing)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (parts, body) = response.into_parts();
        let stream = body
            .into_data_stream()
            .map(|result| result.map_err(|error| io::Error::other(error.to_string())));
        let stream = ResponseStream {
            stream: Box::pin(stream),
            driver: Driver(driver.0.take()),
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
            cancelled: Box::pin(self.factory.0.cancelled.clone().cancelled_owned()),
            ended: false,
            generation: self.factory.0.clone(),
        };
        Ok(reqwest::Response::from(http::Response::from_parts(
            parts,
            reqwest::Body::wrap_stream(stream),
        )))
    }

    pub(super) async fn join(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

impl HttpTransport for ControllerHttp {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        Box::pin(async move {
            let deadline = Instant::now() + Duration::from_secs(10);
            tokio::select! {
                biased;
                _ = self.factory.0.cancelled.cancelled() => Err(HttpError::Io(closed())),
                result = tokio::time::timeout_at(deadline, self.request(request, deadline)) => match result {
                    Ok(result) => result.map_err(HttpError::Io),
                    Err(_) => Err(HttpError::Timeout),
                }
            }
        })
    }
}

struct Driver(Option<JoinHandle<()>>);
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

struct ResponseStream {
    stream: Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send>>,
    driver: Driver,
    deadline: Pin<Box<Sleep>>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    ended: bool,
    generation: Arc<super::generation::Generation>,
}

impl Stream for ResponseStream {
    type Item = io::Result<Bytes>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        let error = if this.cancelled.as_mut().poll(cx).is_ready() {
            Some(closed())
        } else if this.deadline.as_mut().poll(cx).is_ready() {
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "controller response timeout",
            ))
        } else {
            None
        };
        if let Some(error) = error {
            this.ended = true;
            if let Some(task) = this.driver.0.take() {
                task.abort();
            }
            return Poll::Ready(Some(Err(error)));
        }
        let generation = this.generation.clone();
        let state = match generation.state.lock() {
            Ok(state) if state.active => state,
            _ => {
                this.ended = true;
                if let Some(task) = this.driver.0.take() {
                    task.abort();
                }
                return Poll::Ready(Some(Err(closed())));
            }
        };
        let result = this.stream.as_mut().poll_next(cx);
        drop(state);
        if matches!(result, Poll::Ready(None) | Poll::Ready(Some(Err(_)))) {
            this.ended = true;
            if let Some(task) = this.driver.0.take() {
                task.abort();
            }
        }
        result
    }
}
