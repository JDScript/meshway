use meshway_protocol::ipc::{Phase, Status};
use std::io::{self, Write};

mod ping;
pub(crate) use ping::display as ping;

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
        writeln!(
            output,
            "Sync: {:?}  last sync: {}",
            status.sync,
            status
                .last_sync_at
                .map(|time| time.to_rfc3339())
                .unwrap_or_else(|| "none".into())
        )?;
        if let Some(error) = status.sync_error {
            writeln!(output, "Sync error: {error:?}  (retrying)")?;
        }
        match &status.netmap {
            Some(netmap) => writeln!(
                output,
                "Netmap: revision {} serial {}  lease until {}",
                netmap.netmap.revision,
                netmap.netmap.serial.get(),
                netmap.expires_at.to_rfc3339()
            )?,
            None => writeln!(output, "Netmap: none")?,
        }
        if let Some(identity) = &status.identity {
            writeln!(
                output,
                "Control key generation: {}",
                identity.keys.control.generation.get()
            )?;
            writeln!(
                output,
                "Network: {}  Node: {}",
                identity.network_id, identity.node_id
            )?;
        }
        if let Some(url) = &status.enrollment_url {
            writeln!(output, "Approve this device in your browser:\n{url}")?;
        } else if status.phase == Phase::AwaitingApproval {
            writeln!(
                output,
                "Awaiting approval. The Server has not configured a Console origin; ask its operator to configure --console-origin (MESHWAY_CONSOLE_ORIGIN)."
            )?;
        }
        if let Some(saved) = &status.saved {
            writeln!(
                output,
                "Saved netmap: revision {} serial {}",
                saved.revision,
                saved.serial.get()
            )?;
        }
        writeln!(
            output,
            "Network mode: {:?}  Data plane: {:?}",
            status.network_mode, status.data_plane
        )?;
        exit(&mut output, status.exit.as_ref())?;
        if let Some(applied) = &status.applied {
            writeln!(
                output,
                "Applied netmap: revision {} serial {}",
                applied.revision,
                applied.serial.get()
            )?;
        }
        if let Some(error) = status.last_error {
            writeln!(output, "Last problem: {error:?}")?;
        }
    }
    Ok(())
}

fn exit(
    output: &mut impl Write,
    exit: Option<&meshway_protocol::ipc::ExitState>,
) -> io::Result<()> {
    let Some(exit) = exit else {
        return writeln!(output, "Exit: not observed");
    };
    let capability = exit
        .capability
        .as_ref()
        .map(|c| format!("{:?}", c.status))
        .unwrap_or_else(|| "not requested".into());
    let role = exit
        .installed_role
        .as_ref()
        .map(|role| format!("{role}"))
        .unwrap_or_else(|| "none".into());
    let flows = exit
        .active_flows
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unknown".into());
    writeln!(output, "Exit: {capability}  role={role}  flows={flows}")?;
    if let Some(reason) = exit.dns_unavailable {
        writeln!(output, "Exit DNS: unavailable ({reason:?})")?;
    }
    Ok(())
}

/// Build and protocol identity; CLI and daemon of one release share it.
pub(crate) fn version(json: bool) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let git = env!("MESHWAY_BUILD_GIT");
    let target = env!("MESHWAY_BUILD_TARGET");
    let ipc = meshway_protocol::ipc::VERSION;
    let wire = meshway_dataplane::secure::WIRE_VERSION;
    let compiler = meshway_configuration::netmap::NETMAP_COMPILER_VERSION;
    let mut output = io::stdout().lock();
    if json {
        serde_json::to_writer(
            &mut output,
            &serde_json::json!({
                "version": version,
                "git": git,
                "target": target,
                "ipc": ipc,
                "wire": wire,
                "compilerVersion": compiler,
            }),
        )?;
        writeln!(output)
    } else {
        writeln!(output, "meshway {version} ({git})")?;
        writeln!(output, "target:   {target}")?;
        writeln!(output, "ipc:      {ipc}")?;
        writeln!(output, "wire:     {wire}")?;
        writeln!(output, "compiler: {compiler}")
    }
}
