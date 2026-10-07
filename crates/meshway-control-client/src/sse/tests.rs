use super::*;
use crate::Client;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Response},
    routing::get,
};
use futures_util::stream;
use serde::Deserialize;
use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Value {
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Problem {
    code: String,
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn serve(router: Router, limit: usize) -> (Client, reqwest::Url, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let client = Client::new(
        &url,
        crate::http_client_builder()
            .unwrap()
            .no_proxy()
            .timeout(Duration::from_secs(2)),
        limit,
    )
    .unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        client,
        format!("{url}/events").parse().unwrap(),
        Server(task),
    )
}
fn contract() -> StreamResponse<'static, Problem> {
    StreamResponse::json(200, &[401])
}
fn body(bytes: &'static [u8]) -> Body {
    // Force arbitrary UTF-8 and CRLF boundaries; reqwest may coalesce them again.
    Body::from_stream(stream::iter(
        bytes
            .iter()
            .map(|b| Ok::<_, Infallible>(Bytes::copy_from_slice(&[*b]))),
    ))
}

#[tokio::test]
async fn streaming_heartbeats_outlive_ordinary_budget_but_read_idle_still_expires() {
    let router = Router::new()
        .route(
            "/events",
            get(|| async {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(stream::unfold(0, |tick| async move {
                        if tick > 10 {
                            return None;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        Some((
                            Ok::<_, Infallible>(if tick == 10 {
                                Bytes::from_static(b"data: {\"text\":\"alive\"}\n\n")
                            } else {
                                Bytes::from_static(b": keepalive\n\n")
                            }),
                            tick + 1,
                        ))
                    })))
                    .unwrap()
            }),
        )
        .route(
            "/idle",
            get(|| async {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(stream::pending::<
                        Result<Bytes, Infallible>,
                    >()))
                    .unwrap()
            }),
        )
        .route(
            "/ordinary",
            get(|| async {
                Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from_stream(stream::unfold((), |()| async {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        Some((Ok::<_, Infallible>(Bytes::from_static(b" ")), ()))
                    })))
                    .unwrap()
            }),
        );
    let (_, url, _server) = serve(router, 1024).await;
    let client = Client::new(
        &format!("{}/", url.origin().ascii_serialization()),
        crate::http_client_builder()
            .unwrap()
            .no_proxy()
            .read_timeout(Duration::from_millis(150)),
        1024,
    )
    .unwrap()
    .with_request_timeout(Duration::from_millis(60));
    let mut events = client
        .stream::<Value, Problem>(url.clone(), None, &StreamOptions::default(), contract())
        .await
        .unwrap();
    assert_eq!(events.next().await.unwrap().unwrap().data.text, "alive");
    let mut idle = client
        .stream::<Value, Problem>(
            url.join("/idle").unwrap(),
            None,
            &StreamOptions::default(),
            contract(),
        )
        .await
        .unwrap();
    assert!(matches!(
        idle.next().await,
        Some(Err(ClientError::Network { status: Some(200) }))
    ));
    assert!(idle.next().await.is_none());
    let started = std::time::Instant::now();
    let ordinary = client
        .send_url::<(), Problem>(
            reqwest::Method::GET,
            url.join("/ordinary").unwrap(),
            None,
            None,
        )
        .await;
    assert!(matches!(ordinary, Err(ClientError::Network { .. })));
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[tokio::test]
async fn fragmented_frames_are_typed_and_only_explicit_applied_cursor_is_sent() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let received = seen.clone();
    let router = Router::new().route("/events", get(move |headers: HeaderMap| {
        let received = received.clone();
        async move {
            assert_eq!(headers["authorization"], "Bearer private-token");
            assert_eq!(headers["accept"], "text/event-stream");
            received.lock().unwrap().push(headers["last-event-id"].to_str().unwrap().to_owned());
            Response::builder().header("content-type", "text/event-stream; charset=utf-8")
                .body(body("\u{feff}: keepalive\r\nretry: 1\r\nid: decoded-only\r\ndata: {\r\ndata: \"text\":\"😀\"}\r\n\r\ndata: {\"text\":\"snapshot\"}\n\ndata: {\"text\":\"incomplete\"}".as_bytes())).unwrap()
        }
    }));
    let (client, url, _server) = serve(router, 1024).await;
    let options = StreamOptions {
        applied_cursor: Some("applied"),
        ..Default::default()
    };
    for _ in 0..2 {
        let mut events = client
            .stream::<Value, Problem>(url.clone(), Some("private-token"), &options, contract())
            .await
            .unwrap();
        let first = events.next().await.unwrap().unwrap();
        assert_eq!(first.data.text, "😀");
        assert_eq!(first.id.as_deref(), Some("decoded-only"));
        let second = events.next().await.unwrap().unwrap();
        assert_eq!(second.data.text, "snapshot");
        assert!(second.id.is_none());
        assert!(events.next().await.is_none());
        assert_eq!(events.retry_delay(), Some(Duration::from_millis(100)));
        assert!(events.next().await.is_none());
    }
    assert_eq!(*seen.lock().unwrap(), ["applied", "applied"]);
}

#[tokio::test]
async fn malformed_and_over_budget_events_terminate_without_raw_content() {
    for (bytes, limit, expected) in [
        (
            &b"data: {\"text\":\"secret\",\"extra\":true}\n\n"[..],
            1024,
            "Decode",
        ),
        (&b"data: \xff\n\n"[..], 1024, "Decode"),
        (
            &b"event: other\ndata: {\"text\":\"secret\"}\n\n"[..],
            1024,
            "Decode",
        ),
        (&b"data: {\"text\":\"secret\"}\n\n"[..], 8, "BodyLimit"),
        (&b"data: {}\ndata: {}\ndata: {}\n\n"[..], 8, "BodyLimit"),
    ] {
        let router = Router::new().route(
            "/events",
            get(move || async move {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(body(bytes))
                    .unwrap()
            }),
        );
        let (client, url, _server) = serve(router, limit).await;
        let mut events = client
            .stream::<Value, Problem>(url, None, &StreamOptions::default(), contract())
            .await
            .unwrap();
        let error = events.next().await.unwrap().err().unwrap();
        assert!(format!("{error:?}").starts_with(expected));
        assert!(!format!("{error}").contains("secret"));
        assert!(events.next().await.is_none());
    }
}

#[tokio::test]
async fn errors_and_headers_follow_the_generated_status_contract() {
    let router = Router::new().route(
        "/events",
        get(|| async {
            Response::builder()
                .status(401)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"code":"private-problem"}"#))
                .unwrap()
        }),
    );
    let (client, url, _server) = serve(router, 1024).await;
    let result = client
        .stream::<Value, Problem>(url.clone(), None, &StreamOptions::default(), contract())
        .await;
    match result {
        Err(ClientError::Http {
            status: 401,
            problem: Some(p),
        }) => assert_eq!(p.code, "private-problem"),
        _ => panic!("expected typed HTTP problem"),
    }
    for cursor in ["line\nbreak", "non ascii 😀"] {
        assert!(matches!(
            client
                .stream::<Value, Problem>(
                    url.clone(),
                    None,
                    &StreamOptions {
                        applied_cursor: Some(cursor),
                        ..Default::default()
                    },
                    contract()
                )
                .await,
            Err(ClientError::Configuration)
        ));
    }
    assert!(matches!(
        client
            .stream::<Value, Problem>(
                "http://other.invalid/events".parse().unwrap(),
                Some("private-token"),
                &StreamOptions::default(),
                contract()
            )
            .await,
        Err(ClientError::Configuration)
    ));
}

struct Pending {
    dropped: Arc<AtomicBool>,
}
impl futures_util::Stream for Pending {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn dropping_stream_cancels_pending_read_and_wrong_media_rejects_at_headers() {
    for media in ["text/event-stream", "application/json"] {
        let dropped = Arc::new(AtomicBool::new(false));
        let observed = dropped.clone();
        let router = Router::new().route(
            "/events",
            get(move || {
                let dropped = observed.clone();
                async move {
                    Response::builder()
                        .header("content-type", media)
                        .body(Body::from_stream(Pending { dropped }))
                        .unwrap()
                }
            }),
        );
        let (client, url, _server) = serve(router, 1024).await;
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            client.stream::<Value, Problem>(url, None, &StreamOptions::default(), contract()),
        )
        .await
        .expect("headers must resolve without waiting for body");
        if media == "text/event-stream" {
            let mut events = result.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(25), events.next())
                    .await
                    .is_err()
            );
            assert!(!dropped.load(Ordering::SeqCst));
            drop(events);
        } else {
            assert!(matches!(result, Err(ClientError::Decode { status: 200 })));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("response must be released");
    }
}
