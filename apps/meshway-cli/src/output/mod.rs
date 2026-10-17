//! Every command prints either human text (styled only on a TTY) or, with
//! `--json`, one typed JSON document (`ping`: one event per line).
use meshway_protocol::ipc::Status;
use std::io::{self, Write};

pub(crate) mod exit;
mod format;
pub(crate) mod netcheck;
pub(crate) mod ping;
pub(crate) mod status;
mod table;
mod term;
pub(crate) use term::Term;

pub(crate) fn json(output: &mut impl Write, value: &impl serde::Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)
}

/// One status snapshot; a TTY watch redraws the screen in place.
pub(crate) fn status(status: &Status, json: bool, watch: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        self::json(&mut output, status)?;
        return output.flush();
    }
    let term = Term::stdout();
    if watch && term.rich {
        write!(output, "\u{1b}[H\u{1b}[2J")?;
    }
    self::status::write(&mut output, &term, status, chrono::Utc::now())?;
    if watch && !term.rich {
        writeln!(output)?;
    }
    output.flush()
}

/// `meshway exit` without a profile: the netmap's selection.
pub(crate) fn exit_view(
    status: &Status,
    selection: &meshway_configuration::selection::RoutingSelectionState,
    json: bool,
) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        return self::json(&mut output, selection);
    }
    exit::write(
        &mut output,
        &Term::stdout(),
        selection,
        &exit::Names(&status.peers),
    )
}

/// `meshway exit <profile>`: the Server's committed selection and, once
/// applied, the netmap's. JSON keeps both and whether the Node applied it.
pub(crate) fn exit_selected(
    status: &Status,
    committed: &meshway_configuration::selection::RoutingSelectionState,
    applied: bool,
    cleared: bool,
    json: bool,
) -> io::Result<()> {
    let mut output = io::stdout().lock();
    let local = status.netmap.as_ref().and_then(|n| n.routing.as_ref());
    if json {
        return self::json(
            &mut output,
            &serde_json::json!({ "committed": committed, "applied": applied, "local": local, "cleared": cleared }),
        );
    }
    let term = Term::stdout();
    let shown = if applied {
        local.unwrap_or(committed)
    } else {
        committed
    };
    exit::write(&mut output, &term, shown, &exit::Names(&status.peers))?;
    if cleared {
        writeln!(output, "\n{}", exit::cleared(&term, shown))?;
    }
    if !applied {
        writeln!(
            output,
            "\n{}",
            term.paint(
                term::Role::Warn,
                "Saved on the Server; this Node has not applied the new netmap yet."
            )
        )?;
    }
    Ok(())
}

/// The Node after a confirmed self-deletion.
pub(crate) fn logout(before: &Status, after: &Status, json: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        return self::json(&mut output, after);
    }
    let term = Term::stdout();
    let name = before
        .node
        .as_ref()
        .map(|node| node.hostname.clone())
        .or_else(|| {
            before
                .identity
                .as_ref()
                .map(|identity| identity.node_id.to_string())
        })
        .unwrap_or_else(|| "This Node".into());
    let network = before
        .identity
        .as_ref()
        .map(|identity| format!(" network {}", identity.network_id))
        .unwrap_or_default();
    writeln!(
        output,
        "{} {name} left{network}; the Server deleted it and its local identity is removed.",
        term.paint(term::Role::Good, "Logged out:")
    )?;
    writeln!(
        output,
        "Joining again needs a new enrollment: meshway up (browser) or meshway up --enrollment-key."
    )
}

/// Build and protocol identity; CLI and daemon of one release share it.
pub(crate) fn version(json: bool) -> io::Result<()> {
    let version = meshway_build::VERSION;
    let git = meshway_build::GIT;
    let target = meshway_build::TARGET;
    let ipc = meshway_protocol::ipc::VERSION;
    let wire = meshway_dataplane::secure::WIRE_VERSION;
    let compiler = meshway_configuration::netmap::NETMAP_COMPILER_VERSION;
    let mut output = io::stdout().lock();
    if json {
        self::json(
            &mut output,
            &serde_json::json!({
                "version": version,
                "git": git,
                "target": target,
                "ipc": ipc,
                "wire": wire,
                "compilerVersion": compiler,
            }),
        )
    } else {
        writeln!(output, "meshway {version} ({git})")?;
        writeln!(output, "target:   {target}")?;
        writeln!(output, "ipc:      {ipc}")?;
        writeln!(output, "wire:     {wire}")?;
        writeln!(output, "compiler: {compiler}")
    }
}
