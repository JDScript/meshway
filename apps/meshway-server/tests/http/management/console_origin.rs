//! A Console hosted on its own origin (for example Cloudflare Pages) calls
//! `/api/v1` cross-origin with a bearer token, including the network SSE
//! stream; every other origin and the agent API stay same-origin only.
use super::*;
use meshway_server::accounts::delegation::DelegationClients;

const CONSOLE_ORIGIN: &str = "http://127.0.0.1:4173";
const CLIENT_ORIGIN: &str = "http://localhost:4321";

fn allow_origin(response: &Response) -> Option<&str> {
    response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn the_console_origin_may_call_the_user_api_and_its_event_stream_cross_origin() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(dir.path().canonicalize().unwrap().join("controller"));
    settings.console_origin = Some(meshway_server::console_origin(CONSOLE_ORIGIN).unwrap());
    settings.delegation_clients =
        DelegationClients::parse([format!("site={CLIENT_ORIGIN}").as_str()]).unwrap();
    let s = Server::start_with(settings).await;
    let c = client();
    let owner = register(&c, &s, "console-origin@example.test", "valid password").await;
    let token = owner.token.expose();
    let network = create(&c, &s, token, "cors").await.network.id;
    let events = net(&s, network, "events");

    let preflight = |origin: &str, url: String, method: &str, headers: &str| {
        c.request(reqwest::Method::OPTIONS, url)
            .header("Origin", origin)
            .header("Access-Control-Request-Method", method)
            .header("Access-Control-Request-Headers", headers)
            .send()
    };
    for (url, method, headers) in [
        (s.auth("login"), "POST", "content-type"),
        (events.clone(), "GET", "authorization, last-event-id"),
        (
            format!("{}/api/v1/networks", s.url),
            "POST",
            "authorization, content-type",
        ),
    ] {
        let allowed = preflight(CONSOLE_ORIGIN, url.clone(), method, headers)
            .await
            .unwrap();
        assert!(allowed.status().is_success(), "{url}");
        assert_eq!(allow_origin(&allowed), Some(CONSOLE_ORIGIN), "{url}");
        let allow_headers = allowed.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .to_owned();
        for header in headers.split(", ") {
            assert!(allow_headers.contains(header), "{url} {header}");
        }
        let methods = allowed.headers()["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .to_owned();
        for method in ["GET", "POST", "DELETE"] {
            assert!(methods.contains(method), "{methods}");
        }
        assert!(
            allowed
                .headers()
                .get("access-control-allow-credentials")
                .is_none()
        );
    }
    // The delegation client keeps its own access next to the Console.
    let delegated = preflight(CLIENT_ORIGIN, s.auth("session"), "GET", "authorization")
        .await
        .unwrap();
    assert_eq!(allow_origin(&delegated), Some(CLIENT_ORIGIN));
    for (origin, url) in [
        ("https://evil.example", s.auth("login")),
        ("http://127.0.0.1:4174", events.clone()),
        (
            CONSOLE_ORIGIN,
            format!("{}/agent-api/v1/auth/challenges", s.url),
        ),
        (CONSOLE_ORIGIN, format!("{}/openapi.json", s.url)),
    ] {
        let denied = preflight(origin, url.clone(), "POST", "content-type")
            .await
            .unwrap();
        assert_eq!(allow_origin(&denied), None, "{origin} {url}");
    }

    // Actual requests: JSON with the bearer token and the streaming SSE body.
    let session = c
        .get(s.auth("session"))
        .header("Origin", CONSOLE_ORIGIN)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    assert_eq!(allow_origin(&session), Some(CONSOLE_ORIGIN));
    let exposed = session.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        exposed.contains("x-request-id") && exposed.contains("date"),
        "{exposed}"
    );
    let mut stream = c
        .get(&events)
        .header("Origin", CONSOLE_ORIGIN)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(stream.headers()[header::CONTENT_TYPE], "text/event-stream");
    assert_eq!(allow_origin(&stream), Some(CONSOLE_ORIGIN));
    let first = tokio::time::timeout(Duration::from_secs(5), stream.chunk())
        .await
        .expect("bounded SSE delivery")
        .unwrap()
        .expect("stream stays open");
    assert!(!first.is_empty());
    drop(stream);
    let other = c
        .get(s.auth("session"))
        .header("Origin", "https://evil.example")
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::OK);
    assert_eq!(allow_origin(&other), None);
    s.close().await;
}

#[tokio::test]
async fn a_console_origin_alone_enables_cors_for_exactly_that_origin() {
    let _serial = ACCOUNT_FLOW.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::new(dir.path().canonicalize().unwrap().join("controller"));
    settings.console_origin = Some(meshway_server::console_origin(CONSOLE_ORIGIN).unwrap());
    let s = Server::start_with(settings).await;
    let c = client();
    for (origin, expected) in [
        (CONSOLE_ORIGIN, Some(CONSOLE_ORIGIN)),
        (CLIENT_ORIGIN, None),
    ] {
        let response = c
            .get(s.auth("options"))
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(allow_origin(&response), expected, "{origin}");
    }
    s.close().await;
}
