//! CLI waiting and exit semantics; durable state belongs to the daemon.
use crate::{
    arguments::{Arguments, Command as CliCommand},
    output::display,
};
use meshway_ipc::{Client, Command, Phase};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Ipc(#[from] meshway_ipc::ClientError),
    #[error("Cannot write status.")]
    Output(#[from] std::io::Error),
    #[error("Another operator changed Node intent.")]
    IntentChanged,
    #[error("Intent is saved; control convergence is still pending. Use status --watch.")]
    Pending,
    #[error("Intent is saved; inspect the reported control problem.")]
    ControlProblem,
}

pub(crate) async fn run(arguments: Arguments) -> Result<(), Error> {
    let command = match arguments.command {
        CliCommand::Up => "up",
        CliCommand::Down => "down",
        CliCommand::Status { .. } => "status",
    };
    tracing::debug!(command, "Local command started");
    let client = Client::new(arguments.socket);
    let mut status = client.call(Command::Status).await?;
    match arguments.command {
        CliCommand::Status { watch } => loop {
            display(&status, arguments.json)?;
            if !watch {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            status = client.call(Command::Status).await?;
        },
        CliCommand::Up | CliCommand::Down => {
            let up = matches!(arguments.command, CliCommand::Up);
            let command = if up {
                Command::Up {
                    expected_revision: status.revision,
                }
            } else {
                Command::Down {
                    expected_revision: status.revision,
                }
            };
            status = client.call(command).await?;
            if status.desired_up != up {
                return Err(Error::IntentChanged);
            }
            let deadline = Instant::now() + Duration::from_secs(30);
            while matches!(
                status.phase,
                Phase::Starting | Phase::Synchronizing | Phase::Stopping
            ) {
                if Instant::now() >= deadline {
                    display(&status, arguments.json)?;
                    return Err(Error::Pending);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                status = client.call(Command::Status).await?;
                if status.desired_up != up {
                    return Err(Error::IntentChanged);
                }
            }
            display(&status, arguments.json)?;
            if matches!(status.phase, Phase::Blocked | Phase::Retrying) {
                return Err(Error::ControlProblem);
            }
            Ok(())
        }
    }
}
