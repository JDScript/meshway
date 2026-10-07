//! CLI waiting and exit semantics; durable state belongs to the daemon.
use crate::{
    arguments::{Arguments, Command as CliCommand},
    output::display,
};
use meshway_ipc::{Client, Command, Phase, PingEvent, PingTarget};
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
    #[error("Intent is saved; Node convergence is still pending. Use status --watch.")]
    Pending,
    #[error("Intent is saved; inspect the reported Node problem.")]
    ControlProblem,
    #[error("Ping finished without a successful reply.")]
    PingUnsuccessful,
    #[error("Ping failed: {0:?}.")]
    PingFailed(meshway_ipc::PingError),
    #[error(
        "Cannot read enrollment key input; use a valid key, readable @file, or piped stdin (-)."
    )]
    EnrollmentKeyInput,
}

pub(crate) async fn run(arguments: Arguments) -> Result<(), Error> {
    let command = match arguments.command {
        CliCommand::Up { .. } => "up",
        CliCommand::RotateKey => "rotate-key",
        CliCommand::Down => "down",
        CliCommand::Status { .. } => "status",
        CliCommand::Ping { .. } => "ping",
    };
    tracing::debug!(command, "Local command started");
    let client = Client::new(arguments.socket);
    if let CliCommand::Ping { target, count } = arguments.command {
        let target = match target.parse::<std::net::IpAddr>() {
            Ok(address) => PingTarget::MeshIp {
                address: address.to_string(),
            },
            Err(_) => PingTarget::Name { name: target },
        };
        let mut stream = client.ping(target, count).await?;
        while let Some(event) = stream.next().await? {
            crate::output::ping(&event, arguments.json)?;
            match event {
                PingEvent::Completed { success, .. } => {
                    return if success > 0 {
                        Ok(())
                    } else {
                        Err(Error::PingUnsuccessful)
                    };
                }
                PingEvent::Failed { code } => return Err(Error::PingFailed(code)),
                _ => {}
            }
        }
        return Err(Error::PingUnsuccessful);
    }
    let mut status = client.call(Command::Status).await?;
    match arguments.command {
        CliCommand::RotateKey => {
            status = client
                .call(Command::RotateKey {
                    expected_revision: status.revision,
                })
                .await?;
            display(&status, arguments.json)?;
            Ok(())
        }
        CliCommand::Ping { .. } => {
            unreachable!("ping is handled without reading or mutating Node intent")
        }
        CliCommand::Status { watch } => loop {
            display(&status, arguments.json)?;
            if !watch {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            status = client.call(Command::Status).await?;
        },
        CliCommand::Up { .. } | CliCommand::Down => {
            let up = matches!(arguments.command, CliCommand::Up { .. });
            let enrollment_key = match arguments.command {
                CliCommand::Up {
                    enrollment_key: Some(value),
                } => Some(enrollment_key_input(value).await?),
                _ => None,
            };
            let command = if up {
                Command::Up {
                    expected_revision: status.revision,
                    enrollment_key,
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

async fn enrollment_key_input(value: String) -> Result<meshway_ipc::EnrollmentKey, Error> {
    use std::io::Read;
    let mut input = String::new();
    if value == "-" || value == "@-" {
        use std::os::fd::AsFd;
        let file = std::fs::File::from(
            std::io::stdin()
                .as_fd()
                .try_clone_to_owned()
                .map_err(|_| Error::EnrollmentKeyInput)?,
        );
        if file
            .metadata()
            .map_err(|_| Error::EnrollmentKeyInput)?
            .is_file()
        {
            file.take(4097)
                .read_to_string(&mut input)
                .map_err(|_| Error::EnrollmentKeyInput)?;
        } else {
            // Tokio's audited pipe adapter sets nonblocking mode. No detached
            // blocking stdin worker can hold up Ctrl-C or runtime shutdown.
            use tokio::io::AsyncReadExt;
            tokio::net::unix::pipe::Receiver::from_file(file)
                .map_err(|_| Error::EnrollmentKeyInput)?
                .take(4097)
                .read_to_string(&mut input)
                .await
                .map_err(|_| Error::EnrollmentKeyInput)?;
        }
    } else if let Some(path) = value.strip_prefix('@') {
        std::fs::File::open(path)
            .map_err(|_| Error::EnrollmentKeyInput)?
            .take(4097)
            .read_to_string(&mut input)
            .map_err(|_| Error::EnrollmentKeyInput)?;
    } else {
        input = value;
    }
    if input.len() > 4096 {
        return Err(Error::EnrollmentKeyInput);
    }
    meshway_ipc::EnrollmentKey::new(input.trim().to_owned()).map_err(|_| Error::EnrollmentKeyInput)
}
