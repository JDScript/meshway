use meshway_ipc::Status;
use std::io::{self, Write};

pub(crate) fn display(status: &Status, json: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut output, status)?;
        writeln!(output)?;
    } else {
        writeln!(
            output,
            "Node: {:?}  desired={}  authenticated={}",
            status.phase,
            if status.desired_up { "up" } else { "down" },
            status.authenticated
        )?;
        if let Some(identity) = &status.identity {
            writeln!(
                output,
                "Network: {}  Node: {}",
                identity.network_id, identity.node_id
            )?;
        }
        if let Some(url) = &status.enrollment_url {
            writeln!(output, "Approve this device in your browser:\n{url}")?;
        }
        if let Some(saved) = &status.saved {
            writeln!(output, "Saved configuration: revision {}", saved.revision)?;
        }
        writeln!(
            output,
            "Network mode: {:?}  Data plane: {:?}",
            status.network_mode, status.data_plane
        )?;
        if let Some(applied) = &status.applied {
            writeln!(
                output,
                "Applied configuration: revision {}",
                applied.revision
            )?;
        }
        if let Some(error) = status.last_error {
            writeln!(output, "Last problem: {error:?}")?;
        }
    }
    Ok(())
}
