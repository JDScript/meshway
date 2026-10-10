//! Bounded request counts from the real daemon traffic, without credentials,
//! bodies, or an additional source of control operations.
use crate::{Cli, Result};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone)]
pub struct Audit {
    started: Instant,
    minutes: Arc<Mutex<BTreeMap<u64, BTreeMap<&'static str, u64>>>>,
    results: Arc<Mutex<Vec<(u16, meshway_protocol::observations::ObservationResult)>>>,
}

impl Audit {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            minutes: Default::default(),
            results: Default::default(),
        }
    }

    pub async fn save(&self, cli: &Cli) -> Result<()> {
        let results =
            serde_json::to_vec_pretty(&*self.results.lock().map_err(|_| "result audit poisoned")?)?;
        tokio::fs::write(cli.evidence.join("observation-results.json"), results).await?;
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Report {
            elapsed_seconds: u64,
            query_limit_per_minute: u64,
            passed: bool,
            minutes: BTreeMap<u64, BTreeMap<&'static str, u64>>,
        }
        // Three Managers at (30 acquisition + 30 observation)/minute plus the
        // unchanged 30-request startup/restart boundary allowance.
        // This catches the previous unconditional ~360/minute loop while all
        // startup, revocation, outage and recovery stages remain in the sample.
        let minutes = self
            .minutes
            .lock()
            .map_err(|_| "request audit poisoned")?
            .clone();
        let report = Report {
            elapsed_seconds: self.started.elapsed().as_secs(),
            query_limit_per_minute: 210,
            passed: minutes.values().all(|counts| {
                counts.get("udp/query").copied().unwrap_or(0) <= 210
                    && counts.get("obsolete").copied().unwrap_or(0) == 0
            }),
            minutes,
        };
        tokio::fs::write(
            cli.evidence.join("control-requests.json"),
            serde_json::to_vec_pretty(&report)?,
        )
        .await?;
        if !report.passed {
            return Err("control-plane candidate request budget exceeded".into());
        }
        Ok(())
    }
}

pub async fn observe(State(audit): State<Audit>, request: Request, next: Next) -> Response {
    let is_result = request.uri().path().starts_with("/agent-api/v1/tasks/")
        && request.uri().path().ends_with("/results");
    let path = request.uri().path();
    let kind = match path {
        "/agent-api/v1/connectivity/udp/query" => Some("udp/query"),
        "/agent-api/v1/connectivity/udp/attempts" => Some("udp/attempts"),
        "/agent-api/v1/sync" => Some("sync"),
        "/agent-api/v1/events"
        | "/agent-api/v1/configuration/refresh"
        | "/agent-api/v1/configuration/query"
        | "/agent-api/v1/dns/query" => Some("obsolete"),
        "/agent-api/v1/state" => Some("state"),
        _ if path.starts_with("/agent-api/v1/demands") => Some("obsolete"),
        _ => None,
    };
    if let Some(kind) = kind {
        let minute = audit.started.elapsed().as_secs() / 60;
        if let Ok(mut minutes) = audit.minutes.lock() {
            // Fixture has a global deadline; cap storage independently too.
            *minutes
                .entry(minute.min(60))
                .or_default()
                .entry(kind)
                .or_default() += 1;
        }
    }
    if is_result {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, 16 * 1024 * 1024).await else {
            return Response::builder()
                .status(413)
                .body(Body::empty())
                .expect("fixed response");
        };
        // Typed public results only, never credential headers or arbitrary
        // request bodies. Bounded private evidence diagnoses actual rejections.
        let parsed =
            serde_json::from_slice::<meshway_protocol::observations::ObservationResult>(&bytes)
                .ok();
        let response = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        if let Some(parsed) = parsed
            && let Ok(mut results) = audit.results.lock()
            && results.len() < 16
        {
            results.push((response.status().as_u16(), parsed));
        }
        response
    } else {
        next.run(request).await
    }
}
