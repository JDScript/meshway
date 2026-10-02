//! Test-only public evidence, retained on success and panic when requested.
use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use meshway_protocol::{ReportConnectionRuntimeRequest, ReportLatencySamplesRequest};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Serialize)]
#[serde(tag = "kind", content = "body")]
enum PublicRequest {
    Runtime(ReportConnectionRuntimeRequest),
    History(ReportLatencySamplesRequest),
}
#[derive(Serialize)]
struct Event {
    at_ms: u128,
    node: &'static str,
    status: u16,
    request: PublicRequest,
}
pub(super) struct Recorder {
    pub tokens: [Arc<Mutex<Option<String>>>; 2],
    start: Instant,
    events: Mutex<Vec<Event>>,
}
impl Recorder {
    pub async fn forward(&self, request: Request, next: Next) -> Response {
        let node = self.tokens.iter().enumerate().find_map(|(index, token)| {
            token.lock().unwrap().as_ref().and_then(|token| {
                (request
                    .headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    == Some(format!("Bearer {token}").as_str()))
                .then_some(if index == 0 { "a" } else { "b" })
            })
        });
        let path = request.uri().path();
        let runtime = path == "/api/v1/agent/connections/runtime";
        let history = path == "/api/v1/agent/latency/samples";
        if let Some(node) = node
            && request.method() == axum::http::Method::POST
            && (runtime || history)
        {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 256 * 1024).await.unwrap();
            let public = if runtime {
                PublicRequest::Runtime(serde_json::from_slice(&bytes).unwrap())
            } else {
                PublicRequest::History(serde_json::from_slice(&bytes).unwrap())
            };
            let response = next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
            self.events.lock().unwrap().push(Event {
                at_ms: self.start.elapsed().as_millis(),
                node,
                status: response.status().as_u16(),
                request: public,
            });
            response
        } else {
            next.run(request).await
        }
    }
}

pub(super) struct Evidence {
    pub recorder: Arc<Recorder>,
    directory: PathBuf,
    label: &'static str,
}
impl Evidence {
    pub fn new(directory: &Path, label: &'static str) -> Self {
        Self {
            recorder: Arc::new(Recorder {
                tokens: [Arc::default(), Arc::default()],
                start: Instant::now(),
                events: Mutex::new(Vec::new()),
            }),
            directory: directory.to_owned(),
            label,
        }
    }
    pub fn set_peer(&self, path: &Path) {
        // Local disk identity, not a duplicated HTTP contract. Never serialize it.
        #[derive(serde::Deserialize)]
        struct Identity {
            node_credential: String,
        }
        let identity: Identity = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        *self.recorder.tokens[1].lock().unwrap() = Some(identity.node_credential);
    }
}
impl Drop for Evidence {
    fn drop(&mut self) {
        let output = std::env::var_os("MESHWAY_HISTORY_TEST_EVIDENCE").map(PathBuf::from);
        if let Some(output) = &output {
            let _ = std::fs::create_dir_all(output);
            if let Ok(bytes) = serde_json::to_vec_pretty(&*self.recorder.events.lock().unwrap()) {
                let _ = std::fs::write(
                    output.join(format!("{}-controller-events.json", self.label)),
                    bytes,
                );
            }
        }
        for name in ["a.log", "b.log", "a-restart.log"] {
            if let Ok(mut public) = std::fs::read_to_string(self.directory.join(name)) {
                for token in &self.recorder.tokens {
                    if let Some(token) = token.lock().unwrap().as_ref() {
                        public = public.replace(token, "[redacted-node-credential]");
                    }
                }
                if let Some(output) = &output {
                    let _ = std::fs::write(output.join(format!("{}-{name}", self.label)), &public);
                } else if std::thread::panicking() {
                    eprintln!("public history child log {name}: {public}");
                }
            }
        }
    }
}
