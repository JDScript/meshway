//! CLI waiting and exit semantics; durable state belongs to the daemon.
use crate::{
    arguments::{Arguments, Command as CliCommand},
    output,
};
use meshway_protocol::ipc::{Client, Command, Phase, PingEvent, PingTarget};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Ipc(#[from] meshway_protocol::ipc::ClientError),
    #[error("Cannot write output.")]
    Output(#[from] std::io::Error),
    #[error("Another operator changed Node intent.")]
    IntentChanged,
    #[error("Intent is saved; inspect the reported Node problem.")]
    ControlProblem,
    #[error("Ping finished without a successful reply.")]
    PingUnsuccessful,
    #[error("Ping failed: {0:?}.")]
    PingFailed(meshway_protocol::ipc::PingError),
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
        CliCommand::Logout => "logout",
        CliCommand::Status { .. } => "status",
        CliCommand::Netcheck => "netcheck",
        CliCommand::Ping { .. } => "ping",
        CliCommand::Version => "version",
    };
    tracing::debug!(command, "Local command started");
    if let CliCommand::Version = arguments.command {
        crate::output::version(arguments.json)?;
        return Ok(());
    }
    let client = Client::new(arguments.socket);
    if let CliCommand::Netcheck = arguments.command {
        let report = client.netcheck().await?;
        let mut stdout = std::io::stdout().lock();
        if arguments.json {
            output::json(&mut stdout, &report)?;
        } else {
            // Dial sockets are named after this Node's own netmap peers.
            let peers = client
                .call(Command::Status)
                .await
                .map(|status| status.peers)
                .unwrap_or_default();
            output::netcheck::write(&mut stdout, &output::Term::stdout(), &report, &peers)?;
        }
        return Ok(());
    }
    if let CliCommand::Ping {
        target,
        count,
        verbose,
    } = arguments.command
    {
        let target = match target.parse::<std::net::IpAddr>() {
            Ok(address) => PingTarget::MeshIp {
                address: address.to_string(),
            },
            Err(_) => PingTarget::Name { name: target },
        };
        let term = output::Term::stdout();
        let mut view = output::ping::Ping::new(verbose);
        let mut stream = client.ping(target, count).await?;
        while let Some(event) = stream.next().await? {
            if !arguments.json && view.wants_names(&event) {
                // Relays are named from this Node's own netmap peers; without
                // them the line keeps the Node IDs.
                let peers = client
                    .call(Command::Status)
                    .await
                    .map(|status| status.peers)
                    .unwrap_or_default();
                view.names(&peers);
            }
            {
                use std::io::Write;
                let mut stdout = std::io::stdout().lock();
                if arguments.json {
                    output::ping::json(&mut stdout, &event)?;
                } else {
                    view.write(&mut stdout, &term, &event)?;
                }
                stdout.flush()?;
            }
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
            output::status(&status, arguments.json, false)?;
            Ok(())
        }
        CliCommand::Logout => {
            let after = client
                .call(Command::Logout {
                    expected_revision: status.revision,
                })
                .await?;
            output::logout(&status, &after, arguments.json)?;
            Ok(())
        }
        CliCommand::Ping { .. } | CliCommand::Netcheck | CliCommand::Version => {
            unreachable!("handled without reading or mutating Node intent")
        }
        CliCommand::Status { watch } => loop {
            output::status(&status, arguments.json, watch)?;
            if !watch {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            status = client.call(Command::Status).await?;
        },
        CliCommand::Up { .. } | CliCommand::Down => {
            let up = matches!(arguments.command, CliCommand::Up { .. });
            let (enrollment_key, reenroll) = match arguments.command {
                CliCommand::Up {
                    enrollment_key: Some(value),
                    ..
                } => (Some(enrollment_key_input(value).await?), false),
                CliCommand::Up { reenroll, .. } => (None, reenroll),
                _ => (None, false),
            };
            let command = if up {
                Command::Up {
                    expected_revision: status.revision,
                    enrollment_key,
                    reenroll,
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
            while converging(status.phase) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await;
                status = client.call(Command::Status).await?;
                if status.desired_up != up {
                    return Err(Error::IntentChanged);
                }
            }
            output::status(&status, arguments.json, false)?;
            if status.phase == Phase::Blocked {
                return Err(Error::ControlProblem);
            }
            if converging(status.phase) {
                tracing::warn!(
                    phase = ?status.phase,
                    problem = ?status.last_error,
                    "Intent is saved; Node is converging. Use status --watch."
                );
            }
            Ok(())
        }
    }
}

/// The saved intent is still being applied: a transient retry (network, Server
/// busy, an ordinary first-request failure) is convergence, not failure. Only
/// `Blocked` (identity rejection, storage, contract) needs an operator.
fn converging(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Starting | Phase::Synchronizing | Phase::Stopping | Phase::Retrying | Phase::Leaving
    )
}

async fn enrollment_key_input(
    value: String,
) -> Result<meshway_protocol::ipc::EnrollmentKey, Error> {
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
    meshway_protocol::ipc::EnrollmentKey::new(input.trim().to_owned())
        .map_err(|_| Error::EnrollmentKeyInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_blocked_node_is_an_operator_problem() {
        for phase in [
            Phase::Starting,
            Phase::Synchronizing,
            Phase::Stopping,
            Phase::Retrying,
            Phase::Leaving,
        ] {
            assert!(converging(phase), "{phase:?}");
        }
        for phase in [
            Phase::Down,
            Phase::AwaitingApproval,
            Phase::ControlReady,
            Phase::Running,
            Phase::Blocked,
        ] {
            assert!(!converging(phase), "{phase:?}");
        }
    }
}
