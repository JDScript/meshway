//! One joinable process operation at a time, including abandoned async waiters.
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(in crate::linux::kernel) const BUDGET: Duration = Duration::from_secs(2);
const LIMIT: usize = 2 * 1024 * 1024;
// The worker owns this permit, so dropping and recreating a probe cannot bypass
// the process limit while an abandoned blocking operation is still being reaped.
static PROCESS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone, Copy)]
pub(in crate::linux::kernel) enum Tool {
    Nft,
    Conntrack,
}

impl Tool {
    fn path(self) -> &'static str {
        match self {
            Self::Nft => "/usr/sbin/nft",
            Self::Conntrack => "/usr/sbin/conntrack",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(in crate::linux::kernel) enum Failure {
    #[error("kernel tool I/O failed")]
    Io(#[from] std::io::Error),
    #[error("kernel tool exceeded its resource budget")]
    Limit,
    #[error("kernel tool worker failed")]
    Worker,
}

pub(in crate::linux::kernel) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    #[cfg(test)]
    pub stderr: Vec<u8>,
    #[cfg(test)]
    pub status: std::process::ExitStatus,
}

#[derive(Default)]
pub(in crate::linux::kernel) struct Commands {
    pending: Option<tokio::task::JoinHandle<Result<Output, Failure>>>,
    fence: Option<super::child::Fence>,
}

impl Commands {
    pub fn recorded(fence: super::child::Fence) -> Self {
        Self {
            pending: None,
            fence: Some(fence),
        }
    }
    /// An abandoned waiter leaves the join handle here. The owner must settle
    /// and reconcile its journal before submitting another mutation.
    pub async fn settle(&mut self) -> Result<Option<Output>, Failure> {
        let Some(pending) = &mut self.pending else {
            return Ok(None);
        };
        let result = pending.await;
        self.pending = None;
        result.map_err(|_| Failure::Worker)?.map(Some)
    }

    pub async fn run(
        &mut self,
        tool: Tool,
        arguments: Vec<String>,
        input: Vec<u8>,
    ) -> Result<Output, Failure> {
        // Never silently discard the result of an abandoned mutation.
        if self.pending.is_some() || input.len() > LIMIT || arguments.len() > 64 {
            return Err(Failure::Limit);
        }
        let permit = tokio::time::timeout(BUDGET / 2, PROCESS.acquire())
            .await
            .map_err(|_| Failure::Limit)?
            .map_err(|_| Failure::Worker)?;
        let end = Instant::now() + BUDGET;
        let fence = self.fence.clone();
        self.pending = Some(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            execute(tool, &arguments, &input, end, fence.as_ref())
        }));
        self.settle().await?.ok_or(Failure::Worker)
    }
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    reader.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err(Failure::Limit);
    }
    Ok(bytes)
}

struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        // Also runs if starting/joining a pipe worker panics or fails. This guard
        // lives inside the scoped closure, before scope joins blocked readers.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn execute(
    tool: Tool,
    arguments: &[String],
    input: &[u8],
    end: Instant,
    fence: Option<&super::child::Fence>,
) -> Result<Output, Failure> {
    if Instant::now() >= end {
        return Err(Failure::Limit);
    }
    let mut command = if fence.is_some() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", super::child::GATE, "meshway-exit", tool.path()]);
        command
    } else {
        Command::new(tool.path())
    };
    let mut child = command
        .args(arguments)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().ok_or(Failure::Worker)?;
    let stdout = child.stdout.take().ok_or(Failure::Worker)?;
    let stderr = child.stderr.take().ok_or(Failure::Worker)?;
    std::thread::scope(move |scope| {
        let mut child = Reap(child);
        if let Some(fence) = fence {
            fence.record(child.0.id()).map_err(|_| Failure::Worker)?;
        }
        let writer = scope.spawn(move || {
            let mut stdin = stdin;
            if fence.is_some() {
                stdin.write_all(b"go\n")?;
            }
            stdin.write_all(input)
        });
        let reader = scope.spawn(move || read_bounded(stdout));
        let errors = scope.spawn(move || read_bounded(stderr));
        let status = loop {
            match child.0.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                result => {
                    // An ACK may have been lost. Killing/reaping is not rollback.
                    let _ = child.0.kill();
                    let _ = child.0.wait();
                    break match result {
                        Err(error) => Err(Failure::Io(error)),
                        _ => Err(Failure::Limit),
                    };
                }
            }
        };
        let written = writer.join().map_err(|_| Failure::Worker)?;
        let stdout = reader.join().map_err(|_| Failure::Worker)??;
        let _stderr = errors.join().map_err(|_| Failure::Worker)??;
        let status = status?;
        if Instant::now() > end {
            return Err(Failure::Limit);
        }
        // A command that rejects input may close stdin early; retain its failure
        // status instead of turning it into an unobservable successful write.
        if status.success() {
            written?;
        }
        Ok(Output {
            success: status.success(),
            stdout,
            #[cfg(test)]
            stderr: _stderr,
            #[cfg(test)]
            status,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_tool_output_is_bounded_before_decode() {
        assert_eq!(read_bounded(&b"ok"[..]).unwrap(), b"ok");
        assert!(matches!(
            read_bounded(std::io::repeat(b'x')),
            Err(Failure::Limit)
        ));
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace, conntrack events and NET_ADMIN"]
    async fn exit_command_abandoned_waiter_is_still_bounded_and_joined() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        let mut commands = Commands::default();
        let mut waiter = Box::pin(commands.run(
            Tool::Conntrack,
            vec!["-E".into(), "-f".into(), "ipv4".into()],
            Vec::new(),
        ));
        assert!(futures_util::poll!(waiter.as_mut()).is_pending());
        drop(waiter);
        assert!(commands.pending.is_some());
        let mut competing = Commands::default();
        assert!(matches!(
            competing
                .run(
                    Tool::Nft,
                    vec!["-j".into(), "list".into(), "tables".into()],
                    Vec::new()
                )
                .await,
            Err(Failure::Limit)
        ));
        let settled = tokio::time::timeout(BUDGET + Duration::from_secs(3), commands.settle())
            .await
            .unwrap();
        assert!(matches!(settled, Err(Failure::Limit)));
        assert!(commands.pending.is_none());
        let output = commands
            .run(
                Tool::Nft,
                vec!["-j".into(), "list".into(), "tables".into()],
                Vec::new(),
            )
            .await
            .unwrap();
        assert!(output.success);
    }
}
