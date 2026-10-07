//! Bounded request counts from the real daemon traffic, without credentials,
//! bodies, or an additional source of control operations.
use crate::{Cli, Result};
use axum::{
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
}

impl Audit {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            minutes: Default::default(),
        }
    }

    pub async fn save(&self, cli: &Cli) -> Result<()> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Report {
            elapsed_seconds: u64,
            query_limit_per_minute: u64,
            passed: bool,
            minutes: BTreeMap<u64, BTreeMap<&'static str, u64>>,
        }
        // Three Managers at 30/minute plus startup/restart boundary allowance.
        // This catches the previous unconditional ~360/minute loop while all
        // startup, revocation, outage and recovery stages remain in the sample.
        let minutes = self
            .minutes
            .lock()
            .map_err(|_| "request audit poisoned")?
            .clone();
        let report = Report {
            elapsed_seconds: self.started.elapsed().as_secs(),
            query_limit_per_minute: 120,
            passed: minutes
                .values()
                .all(|counts| counts.get("udp/query").copied().unwrap_or(0) <= 120),
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
    let kind = match request.uri().path() {
        "/agent-api/v1/connectivity/udp/query" => Some("udp/query"),
        "/agent-api/v1/connectivity/udp/attempts" => Some("udp/attempts"),
        "/agent-api/v1/events" => Some("events"),
        "/agent-api/v1/configuration/refresh" => Some("refresh"),
        "/agent-api/v1/state" => Some("state"),
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
    next.run(request).await
}
