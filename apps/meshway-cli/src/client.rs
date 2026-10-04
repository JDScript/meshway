use crate::{Arguments, Command as CliCommand};
use meshway_ipc::{Command, Phase, Request, Response, ResultBody, Status, VERSION};
use meshway_model::RequestId;
use std::{io::Write, path::Path, time::Duration};
use tokio::{net::UnixStream, time::Instant};

fn request(command: Command) -> Result<Request, &'static str> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot create local request ID.")?;
    let request_id = RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid())
        .map_err(|_| "Cannot create local request ID.")?;
    Ok(Request {
        version: VERSION,
        request_id,
        command,
    })
}
async fn call(path: &Path, request: &Request) -> Result<Status, &'static str> {
    tokio::time::timeout(Duration::from_secs(12), async {
        let mut stream = UnixStream::connect(path)
            .await
            .map_err(|_| "Cannot connect to meshwayd; check --socket and local permissions.")?;
        // Verify the connected peer against the socket owner, not a UID claimed
        // in the response. The private socket path is chosen by the operator.
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| "Cannot inspect local socket.")?;
        let peer = stream
            .peer_cred()
            .map_err(|_| "Cannot verify daemon identity.")?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o7777 != 0o600
            || peer.uid() != metadata.uid()
        {
            return Err("Local socket identity or permissions differ.");
        }
        meshway_ipc::write(&mut stream, request)
            .await
            .map_err(|_| "Local request failed.")?;
        let response: Response = meshway_ipc::read(&mut stream)
            .await
            .map_err(|_| "Invalid or incomplete daemon response.")?;
        if response.version != VERSION || response.request_id != request.request_id {
            return Err("Daemon protocol or request identity differs.");
        }
        match response.result {
            ResultBody::Status { status } => Ok(*status),
            ResultBody::Error { code } => Err(match code {
                meshway_ipc::ErrorCode::Version => "Daemon protocol version differs.",
                meshway_ipc::ErrorCode::Conflict | meshway_ipc::ErrorCode::RequestConflict => {
                    "Node intent changed; inspect status and retry the command."
                }
                meshway_ipc::ErrorCode::Busy => "Daemon command queue is busy.",
                meshway_ipc::ErrorCode::Stopping => "Daemon is stopping.",
                meshway_ipc::ErrorCode::Storage => "Daemon could not commit its private state.",
            }),
        }
    })
    .await
    .map_err(|_| "Daemon request timed out; inspect status before retrying.")?
}

fn display(status: &Status, json: bool) -> Result<(), &'static str> {
    let mut output = std::io::stdout().lock();
    if json {
        serde_json::to_writer(&mut output, status).map_err(|_| "Cannot write status.")?;
        writeln!(output).map_err(|_| "Cannot write status.")?;
    } else {
        writeln!(
            output,
            "Node: {:?}  desired={}  authenticated={}",
            status.phase,
            if status.desired_up { "up" } else { "down" },
            status.authenticated
        )
        .map_err(|_| "Cannot write status.")?;
        if let Some(identity) = &status.identity {
            writeln!(
                output,
                "Network: {}  Node: {}",
                identity.network_id, identity.node_id
            )
            .map_err(|_| "Cannot write status.")?;
        }
        if let Some(url) = &status.enrollment_url {
            writeln!(output, "Approve this device in your browser:\n{url}")
                .map_err(|_| "Cannot write approval URL.")?;
        }
        if let Some(saved) = &status.saved {
            writeln!(output, "Saved configuration: revision {}", saved.revision)
                .map_err(|_| "Cannot write status.")?;
        }
        writeln!(
            output,
            "Data plane: not implemented; configuration is not applied."
        )
        .map_err(|_| "Cannot write status.")?;
        if let Some(error) = status.last_error {
            writeln!(output, "Last control problem: {error:?}")
                .map_err(|_| "Cannot write status.")?;
        }
    }
    Ok(())
}

pub async fn run(arguments: Arguments) -> Result<(), &'static str> {
    let mut status = call(&arguments.socket, &request(Command::Status)?).await?;
    match arguments.command {
        CliCommand::Status { watch } => loop {
            display(&status, arguments.json)?;
            if !watch {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            status = call(&arguments.socket, &request(Command::Status)?).await?;
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
            status = call(&arguments.socket, &request(command)?).await?;
            if status.desired_up != up {
                return Err("Another operator changed Node intent.");
            }
            let deadline = Instant::now() + Duration::from_secs(30);
            while matches!(
                status.phase,
                Phase::Starting | Phase::Synchronizing | Phase::Stopping
            ) {
                if Instant::now() >= deadline {
                    display(&status, arguments.json)?;
                    return Err(
                        "Intent is saved; control convergence is still pending. Use status --watch.",
                    );
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                status = call(&arguments.socket, &request(Command::Status)?).await?;
                if status.desired_up != up {
                    return Err("Another operator changed Node intent.");
                }
            }
            display(&status, arguments.json)?;
            if matches!(status.phase, Phase::Blocked | Phase::Retrying) {
                return Err("Intent is saved; inspect the reported control problem.");
            }
            Ok(())
        }
    }
}
