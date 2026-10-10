//! Owned CLI child lifecycle and initial diagnostic-only rendezvous. All event
//! decoding and proof validation use the shared ping evidence module.
use crate::{
    Cli, Error, Result, actor,
    ping::{Identity, OUTPUT_LIMIT, PingEvidence, RUN_LIMIT, Run, Stage, decode_stdout},
    signals::{Addresses, Role, Signal, Signals},
};
use meshway_ipc::PingEvent;
use meshway_model::NodeId;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

pub struct Live {
    child: Child,
    path: PathBuf,
    run: Run,
    started: Instant,
}
impl Live {
    pub async fn start(cli: &Cli, stage: Stage, selector: &str, count: u16) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let before = Identity::from_status(actor::command(cli, "status").await?)?;
        let path = cli.evidence.join(format!("ping-{stage:?}.stdout"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let command = vec![
            cli.binaries.join("meshway").display().to_string(),
            "--socket".into(),
            cli.state.join("node/control.sock").display().to_string(),
            "ping".into(),
            selector.into(),
            "--count".into(),
            count.to_string(),
            "--json".into(),
        ];
        let started_at_ms = crate::recovery::now_ms()?;
        let child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::from(file))
            .stderr(Stdio::from(actor::private_file(
                &cli.evidence.join(format!("ping-{stage:?}.stderr")),
            )?))
            .kill_on_drop(true)
            .spawn()?;
        let child_pid = child.id().ok_or("ping child PID absent")?;
        Ok(Self {
            child,
            path,
            started: Instant::now(),
            run: Run {
                stage,
                command,
                selector: selector.into(),
                child_pid,
                started_at_ms,
                finished_at_ms: 0,
                elapsed_ms: 0,
                exit_code: None,
                exit_signal: None,
                sent_signal: None,
                after: before.clone(),
                before,
                events: Vec::new(),
            },
        })
    }
    async fn read(&self) -> Result<Vec<PingEvent>> {
        let mut bytes = Vec::new();
        tokio::fs::File::open(&self.path)
            .await?
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await?;
        decode_stdout(&bytes)
    }
    pub async fn first_sample(&mut self) -> Result<()> {
        let limit = match crate::remaining(RUN_LIMIT) {
            Ok(limit) => limit,
            Err(error) => {
                actor::kill_join(&mut self.child).await?;
                return Err(error);
            }
        };
        let result = tokio::time::timeout(limit, async {
            loop {
                // A live writer may currently be between the JSON and newline.
                let mut bytes = Vec::new();
                tokio::fs::File::open(&self.path)
                    .await?
                    .take(OUTPUT_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                if bytes.len() as u64 > OUTPUT_LIMIT {
                    return Err("ping stdout exceeds bound".into());
                }
                if let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n')
                    && decode_stdout(&bytes[..=end])?
                        .iter()
                        .any(|event| matches!(event, PingEvent::Sample { sample, .. } if sample.probe.as_ref().is_some_and(|probe| probe.outcome == meshway_ipc::ProbeOutcome::Success)))
                {
                    return Ok::<_, Error>(());
                }
                if self.child.try_wait()?.is_some() {
                    return Err("live ping exited before first sample".into());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        if !matches!(&result, Ok(Ok(()))) {
            actor::kill_join(&mut self.child).await?;
        }
        result?
    }
    pub async fn interrupt(&mut self) -> Result<()> {
        let result = async {
            let pid = self.child.id().ok_or("owned ping already exited")?;
            let mut signal = Command::new("/bin/kill")
                .args(["-INT", &pid.to_string()])
                .kill_on_drop(true)
                .spawn()?;
            let result = tokio::time::timeout(crate::IO, signal.wait()).await;
            if !matches!(&result, Ok(Ok(_))) {
                actor::kill_join(&mut signal).await?;
            }
            if !result??.success() {
                return Err("owned ping SIGINT failed".into());
            }
            self.run.sent_signal = Some(2);
            Ok::<_, Error>(())
        }
        .await;
        if result.is_err() {
            actor::kill_join(&mut self.child).await?;
        }
        result
    }
    pub async fn finish(mut self, cli: &Cli, limit: Duration) -> Result<Run> {
        use std::os::unix::process::ExitStatusExt;
        let limit = match crate::remaining(limit) {
            Ok(limit) => limit,
            Err(error) => {
                actor::kill_join(&mut self.child).await?;
                return Err(error);
            }
        };
        let result = tokio::time::timeout(limit, self.child.wait()).await;
        if !matches!(&result, Ok(Ok(_))) {
            actor::kill_join(&mut self.child).await?;
        }
        let status = result??;
        self.run.exit_code = status.code();
        self.run.exit_signal = status.signal();
        self.run.elapsed_ms = self.started.elapsed().as_millis().try_into()?;
        self.run.finished_at_ms = crate::recovery::now_ms()?;
        self.run.after = Identity::from_status(actor::command(cli, "status").await?)?;
        self.run.events = self.read().await?;
        actor::save(cli, &format!("ping-{:?}", self.run.stage), &self.run).await?;
        Ok(self.run)
    }
    pub async fn abort(mut self) -> Result<()> {
        actor::kill_join(&mut self.child).await
    }
}

pub async fn run(cli: &Cli, stage: Stage, selector: &str, path: &[NodeId]) -> Result<Run> {
    let run = Live::start(cli, stage, selector, 2)
        .await?
        .finish(cli, RUN_LIMIT)
        .await?;
    run.validate(path)?;
    Ok(run)
}

pub async fn initial(
    cli: &Cli,
    signals: &Signals,
    addresses: &BTreeMap<Role, Addresses>,
) -> Result<PingEvidence> {
    signals.ready("ping-go").await?;
    let path = crate::fallback::original_path(signals).await?;
    let mut evidence = PingEvidence::default();
    for (stage, selector) in [
        (Stage::Hostname, "b".into()),
        (Stage::Ipv4, addresses[&Role::B].ipv4.to_string()),
        (Stage::Ipv6, addresses[&Role::B].ipv6.to_string()),
        (Stage::Direct, "r".into()),
    ] {
        let expected = if stage == Stage::Direct {
            &path[..2]
        } else {
            &path
        };
        let run = run(cli, stage, &selector, expected).await?;
        signals
            .write(
                &format!("ping-{stage:?}").to_lowercase(),
                &Signal::PingRun {
                    run: Box::new(run.clone()),
                },
            )
            .await?;
        evidence.runs.push(run);
    }
    signals.ready("ping-shared-go").await?;
    let run = run(cli, Stage::Shared, "b", &path).await?;
    signals
        .write(
            "ping-shared",
            &Signal::PingRun {
                run: Box::new(run.clone()),
            },
        )
        .await?;
    evidence.runs.push(run);
    signals.ready("ping-shared-verified").await?;
    let mut business_packets = 0_u64;
    for event in actor::log_events(cli).await? {
        if crate::fallback::packet_observation(&event)?.is_some() {
            business_packets += 1;
        }
    }
    if business_packets != 0 {
        return Err("business packets existed before initial CLI diagnostics completed".into());
    }
    evidence.diagnostic_only_business_packets = Some(business_packets);
    Ok(evidence)
}
